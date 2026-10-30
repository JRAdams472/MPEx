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

/// Result of a chain pass: rows checked, and on failure the first row that
/// breaks the chain plus why.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainVerify {
    pub checked: i64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_bad_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Recompute every row's hash in id order, checking both linkage
/// (`prev_hash` equals the prior row's `hash`) and content (stored `hash`
/// equals the recomputed digest). A healthy chain returns `ok` with
/// `checked` equal to the row count.
pub async fn verify_chain(pool: &PgPool) -> Result<ChainVerify, AppError> {
    let rows = sqlx::query(
        "SELECT id, occurred_at, actor_sub, action, artifact_table,
                artifact_key, detail, prev_hash, hash
         FROM audit_events ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let mut expected_prev = GENESIS.to_string();
    for (i, row) in rows.iter().enumerate() {
        let id: i64 = row.get("id");
        let prev_hash: String = row.get("prev_hash");
        let hash: String = row.get("hash");
        if prev_hash != expected_prev {
            return Ok(ChainVerify {
                checked: i as i64,
                ok: false,
                first_bad_id: Some(id),
                reason: Some("prev_hash does not match prior row's hash".into()),
            });
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
            return Ok(ChainVerify {
                checked: i as i64,
                ok: false,
                first_bad_id: Some(id),
                reason: Some("stored hash does not match recomputed event hash".into()),
            });
        }
        expected_prev = hash;
    }
    Ok(ChainVerify {
        checked: rows.len() as i64,
        ok: true,
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
