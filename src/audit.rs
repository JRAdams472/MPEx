//! Append-only, hash-chained audit trail (MPE-97 phase 3).
//!
//! Every sensitive mutation writes one attributed `audit_events` row inside
//! the mutation's own transaction, so a rollback leaves no audit row. Each
//! row carries `hash = sha256(prev_hash ‖ canonical(event))`, letting
//! `verify_chain` detect rows rewritten out-of-band. Postgres also enforces
//! append-only storage via an update/delete trigger — the two defenses are
//! complementary: the trigger blocks casual edits through the app role, the
//! chain catches edits made around it.
//!
//! `detail` should stay scalar-safe (strings, integers, booleans): JSONB and
//! serde_json can disagree on floating-point formatting, which would false-
//! flag a verify. Keep payload context in strings/ints.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row as _, Transaction};

use crate::error::AppError;

/// Advisory-lock key serializing chain writers across concurrent
/// transactions. Xact-scoped: released when the caller's tx ends.
const CHAIN_LOCK: i64 = 0x4d50_4558_4155_4449;

/// `prev_hash` sentinel on the first row of the chain.
pub const GENESIS: &str = "";

/// Event names — mirrored by the `action` CHECK constraint in migration 0007.
pub mod action {
    pub const ARTIFACT_PUT: &str = "artifact.put";
    pub const ARTIFACT_DELETE: &str = "artifact.delete";
    pub const PUBLICATION_CREATE: &str = "publication.create";
    pub const AUTH_DENIED: &str = "auth.denied";
    pub const GRANT_PUT: &str = "grant.put";
    pub const GRANT_DELETE: &str = "grant.delete";
    pub const RATE_INDEX_IMPORT: &str = "rate_index.import";
    pub const PRODUCT_RATE_CONFIG_PUT: &str = "product.rate_config.put";
    pub const PROMOTION_CREATE: &str = "promotion.create";
}

/// Attribution for an event, derived from the request's `Principal`.
#[derive(Debug, Clone)]
pub struct Actor {
    pub sub: String,
    pub email: Option<String>,
}

impl From<&crate::auth::Principal> for Actor {
    fn from(p: &crate::auth::Principal) -> Self {
        Actor {
            sub: p.sub.clone(),
            email: p.email.clone(),
        }
    }
}

/// One chain entry before it gets its hash. `detail` is free-form JSONB
/// context; the actor's email is merged in for readable attribution.
#[derive(Debug)]
pub struct Event {
    pub action: &'static str,
    pub artifact_table: Option<&'static str>,
    pub artifact_key: Option<String>,
    pub detail: Value,
}

/// Append `event` to the chain inside `tx`. The advisory lock serializes
/// writers, so `prev_hash` always points at the committed tail — concurrent
/// mutations wait rather than fork the chain.
pub async fn record(
    tx: &mut Transaction<'_, Postgres>,
    actor: impl Into<Actor>,
    event: Event,
) -> Result<(), AppError> {
    let actor = actor.into();
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(CHAIN_LOCK)
        .execute(&mut **tx)
        .await?;
    let prev_hash: String =
        sqlx::query_scalar("SELECT hash FROM audit_events ORDER BY id DESC LIMIT 1")
            .fetch_optional(&mut **tx)
            .await?
            .unwrap_or_default();

    // Truncate to micros before hashing AND binding: timestamptz stores exact
    // microseconds, and verify_chain recomputes the identical canonical form.
    let micros = Utc::now().timestamp_micros();
    let occurred_at =
        DateTime::<Utc>::from_timestamp_micros(micros).expect("micros always in range");

    let mut detail = event.detail;
    if let (Some(email), Value::Object(m)) = (&actor.email, &mut detail) {
        m.insert("actorEmail".into(), Value::from(email.clone()));
    }
    let canonical = canonical(
        &occurred_at,
        &actor.sub,
        event.action,
        event.artifact_table,
        event.artifact_key.as_deref(),
        &detail,
    );
    let hash = chain_hash(&prev_hash, &canonical);

    sqlx::query(
        "INSERT INTO audit_events
           (occurred_at, actor_sub, action, artifact_table, artifact_key,
            detail, prev_hash, hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(occurred_at)
    .bind(&actor.sub)
    .bind(event.action)
    .bind(event.artifact_table)
    .bind(event.artifact_key)
    .bind(&detail)
    .bind(&prev_hash)
    .bind(&hash)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Result of a chain pass: rows checked this pass, the scope walked
/// (`full` from genesis or `incremental` from a watermark), and on failure
/// the first row that breaks the chain plus why.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainVerify {
    pub checked: i64,
    pub ok: bool,
    pub scope: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watermark_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_bad_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Verify the chain, resuming from the newest recorded watermark when one
/// exists (MPE-125). Cost is O(new events): the watermark checkpoint row is
/// recomputed once, then only rows after it are walked. A tampered history
/// surfaces as a checkpoint mismatch (rewrites must propagate forward to
/// stay self-consistent) or a break inside the new rows. A *single-point*
/// content edit before the checkpoint that leaves the chain inconsistent is
/// only visible to `verify_chain_full` — run it after restores or suspected
/// tampering.
pub async fn verify_chain(pool: &PgPool) -> Result<ChainVerify, AppError> {
    verify_chain_scoped(pool, false).await
}

/// Same checks, but always walks the whole chain from genesis — catches
/// pre-watermark inconsistencies an incremental pass cannot see.
pub async fn verify_chain_full(pool: &PgPool) -> Result<ChainVerify, AppError> {
    verify_chain_scoped(pool, true).await
}

async fn verify_chain_scoped(pool: &PgPool, full: bool) -> Result<ChainVerify, AppError> {
    // The newest watermark is the deepest verified tail. `full` ignores it.
    let watermark: Option<(i64, String)> = if full {
        None
    } else {
        sqlx::query(
            "SELECT last_event_id, last_event_hash
             FROM audit_verify_watermarks
             ORDER BY last_event_id DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await?
        .map(|r| {
            (
                r.get::<i64, _>("last_event_id"),
                r.get::<String, _>("last_event_hash"),
            )
        })
    };
    let scope = if watermark.is_some() {
        "incremental"
    } else {
        "full"
    };
    let watermark_id = watermark.as_ref().map(|w| w.0);
    let fail = |checked, first_bad_id, reason: &str| {
        Ok(ChainVerify {
            checked,
            ok: false,
            scope,
            watermark_id,
            first_bad_id: Some(first_bad_id),
            reason: Some(reason.into()),
        })
    };

    let (start_id, mut expected_prev) = match &watermark {
        // Checkpoint self-check: the watermarked row must still exist, still
        // store the recorded hash, and still recompute to it.
        Some((wid, whash)) => {
            let row = sqlx::query(
                "SELECT occurred_at, actor_sub, action, artifact_table,
                        artifact_key, detail, prev_hash, hash
                 FROM audit_events WHERE id = $1",
            )
            .bind(wid)
            .fetch_optional(pool)
            .await?;
            let Some(row) = row else {
                return fail(0, *wid, "watermark checkpoint row is missing");
            };
            let stored: String = row.get("hash");
            if stored != *whash {
                return fail(
                    0,
                    *wid,
                    "watermark checkpoint hash changed — history rewritten",
                );
            }
            let recomputed = chain_hash(
                row.get::<String, _>("prev_hash").as_str(),
                &canonical(
                    &row.get::<DateTime<Utc>, _>("occurred_at"),
                    row.get::<String, _>("actor_sub").as_str(),
                    row.get::<String, _>("action").as_str(),
                    row.get::<Option<String>, _>("artifact_table").as_deref(),
                    row.get::<Option<String>, _>("artifact_key").as_deref(),
                    &row.get::<Value, _>("detail"),
                ),
            );
            if recomputed != stored {
                return fail(
                    0,
                    *wid,
                    "watermark checkpoint content changed — stored hash no longer matches",
                );
            }
            (*wid, whash.clone())
        }
        None => (0, GENESIS.to_string()),
    };

    let rows = sqlx::query(
        "SELECT id, occurred_at, actor_sub, action, artifact_table,
                artifact_key, detail, prev_hash, hash
         FROM audit_events WHERE id > $1 ORDER BY id",
    )
    .bind(start_id)
    .fetch_all(pool)
    .await?;

    for (i, row) in rows.iter().enumerate() {
        let id: i64 = row.get("id");
        let prev_hash: String = row.get("prev_hash");
        let hash: String = row.get("hash");
        if prev_hash != expected_prev {
            return fail(i as i64, id, "prev_hash does not match prior row's hash");
        }
        let occurred_at: DateTime<Utc> = row.get("occurred_at");
        let actor_sub: String = row.get("actor_sub");
        let action: String = row.get("action");
        let artifact_table: Option<String> = row.get("artifact_table");
        let artifact_key: Option<String> = row.get("artifact_key");
        let detail: Value = row.get("detail");
        let canonical = canonical(
            &occurred_at,
            &actor_sub,
            &action,
            artifact_table.as_deref(),
            artifact_key.as_deref(),
            &detail,
        );
        if chain_hash(&prev_hash, &canonical) != hash {
            return fail(
                i as i64,
                id,
                "stored hash does not match recomputed event hash",
            );
        }
        expected_prev = hash;
    }

    // Healthy pass: record the verified tail so the next call starts here.
    // Nothing to watermark when the walk covered zero new rows (empty chain
    // or unchanged tail).
    if let Some(last) = rows.last() {
        sqlx::query(
            "INSERT INTO audit_verify_watermarks
               (last_event_id, last_event_hash, checked, scope)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(last.get::<i64, _>("id"))
        .bind(last.get::<String, _>("hash"))
        .bind(rows.len() as i64)
        .bind(scope)
        .execute(pool)
        .await?;
    }

    Ok(ChainVerify {
        checked: rows.len() as i64,
        ok: true,
        scope,
        watermark_id,
        first_bad_id: None,
        reason: None,
    })
}

/// Deterministic byte form of an event. Unit separators keep any field value
/// from smuggling a delimiter and shifting the parse; `detail` is rendered
/// with recursively sorted keys so JSONB round-trips stay canonical.
fn canonical(
    occurred_at: &DateTime<Utc>,
    actor_sub: &str,
    action: &str,
    artifact_table: Option<&str>,
    artifact_key: Option<&str>,
    detail: &Value,
) -> String {
    [
        occurred_at.to_rfc3339_opts(SecondsFormat::Micros, true),
        actor_sub.to_string(),
        action.to_string(),
        artifact_table.unwrap_or_default().to_string(),
        artifact_key.unwrap_or_default().to_string(),
        canonical_json(detail),
    ]
    .join("\u{1f}")
}

fn chain_hash(prev_hash: &str, canonical: &str) -> String {
    let mut h = Sha256::new();
    h.update(prev_hash.as_bytes());
    h.update(b"\x1e");
    h.update(canonical.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// sha256 hex of a JSON value's canonical form — records *which* document a
/// put wrote without copying the document into the audit row.
pub fn content_hash(v: &Value) -> String {
    let mut h = Sha256::new();
    h.update(canonical_json(v).as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Same for a plain-text payload (e.g. an expression's source).
pub fn content_hash_str(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// JSON rendered with recursively sorted object keys — stable regardless of
/// the `serde_json` map implementation or JSONB's own key ordering.
fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let inner = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        canonical_json(&m[*k])
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical_json).collect::<Vec<_>>().join(",")
        ),
        _ => serde_json::to_string(v).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_json_sorts_nested_keys() {
        let v = json!({"b": 2, "a": {"d": 1, "c": [2, 1]}});
        assert_eq!(canonical_json(&v), r#"{"a":{"c":[2,1],"d":1},"b":2}"#);
    }

    #[test]
    fn chain_hash_is_deterministic() {
        let h = chain_hash("prev", "canon");
        assert_eq!(h.len(), 64);
        assert_eq!(h, chain_hash("prev", "canon"));
        assert_ne!(h, chain_hash("prev2", "canon"));
        assert_ne!(h, chain_hash("prev", "canon2"));
    }
}
