//! MND daily rate-index import (MPE-137): fetch the index page, parse the
//! six base rates, resolve each product's sheet (base via `mnd_label`,
//! derived via `base_product_code` + `rate_adjustment`), then write drafts
//! and stamp a rate-sheets-scoped publication — all in one transaction.
//! The same `import` backs the manual endpoint and the daily scheduler.

pub mod parser;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, NaiveTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

use crate::audit::{self, Actor};
use crate::error::AppError;
use crate::rules::publish::{self, Publication};

/// Default index page — overridable via `RATE_INDEX_URL` (tests point it
/// at a stub server).
pub const DEFAULT_URL: &str = "https://www.mortgagenewsdaily.com/mortgage-rates/mnd";
/// Scheduled imports are attributed to this synthetic actor.
pub const SCHEDULER_SUB: &str = "rate-index-scheduler";
/// The index page is plain HTML but sits behind CDN bot filtering — a
/// browser UA is what the page expects.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// Lock term every imported sheet offers — MND quotes its index on a
/// 30-day basis.
const IMPORT_LOCK_TERM_DAYS: u32 = 30;
/// Sanity bound for an imported or derived rate.
const MAX_RESOLVED_RATE: f64 = 25.0;

/// Where the index lives and how long a fetch may take.
#[derive(Debug, Clone)]
pub struct IndexSettings {
    pub url: String,
    pub timeout: Duration,
}

/// One product's resolved sheet.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedSheet {
    pub product_code: String,
    /// Final note rate written to the sheet.
    pub rate: f64,
    /// `index:<label>` for base products; `base:<code>` for derived ones.
    pub source: String,
    /// The adjustment applied for derived products (0 for base products).
    pub adjustment: f64,
}

/// What `resolve` made of the catalog for one fetched index.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveOutcome {
    pub sheets: Vec<ResolvedSheet>,
    /// Products with no rate source — their sheets stay manual.
    pub skipped: Vec<String>,
    /// Index labels no product maps to (informational).
    pub unmapped: Vec<String>,
}

/// Resolves the catalog against a fetched index. Base products take their
/// `mnd_label` rate directly; derived products take their base product's
/// rate + `rate_adjustment`. A derived product pointing at a non-index
/// base is a config error (the rate-config endpoint can't create one, but
/// direct SQL can) — it fails the resolve rather than silently skip.
pub async fn resolve<'e, E>(
    ex: E,
    rates: &BTreeMap<String, f64>,
) -> Result<ResolveOutcome, AppError>
where
    E: Executor<'e, Database = Postgres>,
{
    let products = sqlx::query_as::<_, (String, Option<String>, Option<String>, f64)>(
        "SELECT code, mnd_label, base_product_code, rate_adjustment
         FROM products WHERE active ORDER BY code",
    )
    .fetch_all(ex)
    .await?;

    // Pass 1: base products anchor to their index label.
    let mut base_rates: HashMap<String, f64> = HashMap::new();
    let mut mapped: HashSet<String> = HashSet::new();
    for (code, mnd_label, _, _) in &products {
        if let Some(label) = mnd_label {
            let rate = *rates.get(label.as_str()).ok_or_else(|| {
                AppError::BadGateway(format!(
                    "product {code} maps to {label:?} but the index did not return it"
                ))
            })?;
            base_rates.insert(code.clone(), rate);
            mapped.insert(label.clone());
        }
    }

    // Pass 2: emit sheets — base first, derived off the base map, manual
    // skipped.
    let mut sheets = Vec::new();
    let mut skipped = Vec::new();
    for (code, mnd_label, base, adjustment) in products {
        if let Some(label) = mnd_label {
            sheets.push(ResolvedSheet {
                rate: rates[label.as_str()],
                product_code: code,
                source: format!("index:{label}"),
                adjustment: 0.0,
            });
        } else if let Some(base) = base {
            let rate = base_rates.get(&base).copied().ok_or_else(|| {
                AppError::Conflict(format!(
                    "product {code} derives from {base}, which is not an index product"
                ))
            })? + adjustment;
            if !(0.0..MAX_RESOLVED_RATE).contains(&rate) {
                return Err(AppError::Conflict(format!(
                    "product {code} resolves to {rate} (base + {adjustment}) — out of range"
                )));
            }
            sheets.push(ResolvedSheet {
                product_code: code,
                rate,
                source: format!("base:{base}"),
                adjustment,
            });
        } else {
            skipped.push(code);
        }
    }

    let unmapped = parser::INDEX_LABELS
        .iter()
        .filter(|l| !mapped.contains(**l))
        .map(|l| (*l).to_string())
        .collect();
    Ok(ResolveOutcome {
        sheets,
        skipped,
        unmapped,
    })
}

/// What one fetch produced, pre-parse.
pub struct Fetched {
    pub html: String,
    pub content_hash: String,
    pub fetched_at: DateTime<Utc>,
}

/// GETs the index page. Any transport/status failure is a 502 — the
/// upstream source misbehaving is not a client error.
pub async fn fetch_index(settings: &IndexSettings) -> Result<Fetched, AppError> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(settings.timeout)
        .build()
        .map_err(|e| AppError::Internal(e.into()))?;
    let res = client
        .get(&settings.url)
        .send()
        .await
        .map_err(|e| AppError::BadGateway(format!("rate index fetch failed: {e}")))?;
    if !res.status().is_success() {
        return Err(AppError::BadGateway(format!(
            "rate index returned HTTP {}",
            res.status()
        )));
    }
    let html = res
        .text()
        .await
        .map_err(|e| AppError::BadGateway(format!("rate index body read failed: {e}")))?;
    let content_hash = Sha256::digest(html.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    Ok(Fetched {
        html,
        content_hash,
        fetched_at: Utc::now(),
    })
}

/// The single-rate-at-par sheet spec an import writes.
fn sheet_spec(rate: f64) -> Value {
    json!({
        "rates": [rate],
        "lockTerms": [IMPORT_LOCK_TERM_DAYS],
        "prices": { format!("{rate}:{IMPORT_LOCK_TERM_DAYS}"): 100.0 },
    })
}

/// The full import: fetch → parse → resolve → upsert drafts → scoped
/// publish → provenance + audit, atomic in one transaction. An unchanged
/// re-import still records `rate_index_imports` but stamps no publication.
#[tracing::instrument(
    name = "rate_index.import",
    skip(pool, actor, settings),
    fields(sheets = tracing::field::Empty, publication = tracing::field::Empty)
)]
pub async fn import(
    pool: &PgPool,
    actor: &Actor,
    settings: &IndexSettings,
) -> Result<PublicationOutcome, AppError> {
    let fetched = fetch_index(settings).await?;
    let rates = parser::parse_index(&fetched.html).map_err(AppError::BadGateway)?;

    let mut tx = pool.begin().await?;
    let outcome = resolve(&mut *tx, &rates).await?;
    if outcome.sheets.is_empty() {
        return Err(AppError::Conflict(
            "no products resolve against the index — configure mnd_label or base_product_code"
                .into(),
        ));
    }

    for sheet in &outcome.sheets {
        sqlx::query(
            "INSERT INTO rate_sheets (key, status, spec)
             VALUES ($1, 'draft', $2)
             ON CONFLICT (key) WHERE status = 'draft'
             DO UPDATE SET spec = EXCLUDED.spec, updated_at = now()",
        )
        .bind(&sheet.product_code)
        .bind(sheet_spec(sheet.rate))
        .execute(&mut *tx)
        .await?;
    }

    let label = format!("MND index {}", fetched.fetched_at.format("%Y-%m-%d"));
    let publication = publish::publish_rate_sheets(&mut tx, &label, actor.clone()).await?;
    let publication_id = publication.as_ref().map(|p| p.id);

    let import_id: Uuid = sqlx::query_scalar(
        "INSERT INTO rate_index_imports
             (fetched_at, source_url, content_hash, rates, imported_by, publication_id)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(fetched.fetched_at)
    .bind(&settings.url)
    .bind(&fetched.content_hash)
    .bind(serde_json::to_value(&rates).unwrap_or_default())
    .bind(&actor.sub)
    .bind(publication_id)
    .fetch_one(&mut *tx)
    .await?;

    audit::record(
        &mut tx,
        actor.clone(),
        audit::Event {
            action: audit::action::RATE_INDEX_IMPORT,
            artifact_table: Some("rate_index_imports"),
            artifact_key: Some(import_id.to_string()),
            detail: json!({
                "sourceUrl": settings.url,
                "contentHash": fetched.content_hash,
                "rates": rates,
                "sheets": outcome.sheets,
                "skipped": outcome.skipped,
                "publicationId": publication_id,
            }),
        },
    )
    .await?;
    tx.commit().await?;

    tracing::Span::current()
        .record("sheets", outcome.sheets.len())
        .record(
            "publication",
            tracing::field::display(
                publication_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "none".into()),
            ),
        );
    Ok(PublicationOutcome {
        import_id,
        fetched_at: fetched.fetched_at,
        content_hash: fetched.content_hash,
        rates,
        outcome,
        publication,
    })
}

/// What an import produced — rendered by the endpoint, logged by the
/// scheduler.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicationOutcome {
    pub import_id: Uuid,
    pub fetched_at: DateTime<Utc>,
    pub content_hash: String,
    pub rates: BTreeMap<String, f64>,
    #[serde(flatten)]
    pub outcome: ResolveOutcome,
    pub publication: Option<Publication>,
}

/// Parses `RATE_INDEX_DAILY_AT` ("HH:MM", UTC).
pub fn parse_daily_at(s: &str) -> Result<NaiveTime, AppError> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M")
        .map_err(|_| AppError::BadRequest(format!("RATE_INDEX_DAILY_AT must be HH:MM, got {s:?}")))
}

/// Seconds from `now` until the next `at` (UTC) — always positive.
pub fn secs_until_daily(now: DateTime<Utc>, at: NaiveTime) -> u64 {
    let today = now.date_naive().and_time(at);
    let next = if today > now.naive_utc() {
        today
    } else {
        today + chrono::Duration::days(1)
    };
    (next - now.naive_utc()).num_seconds().max(1) as u64
}

/// The daily loop: sleep until the next UTC wall-clock time, run the same
/// import the endpoint runs, log and continue on failure — a bad fetch or
/// a changed page must never take the API down.
pub async fn run_daily(pool: PgPool, settings: IndexSettings, at: NaiveTime) {
    let actor = Actor {
        sub: SCHEDULER_SUB.into(),
        email: None,
    };
    loop {
        let wait = secs_until_daily(Utc::now(), at);
        tracing::info!(at = %at, wait_secs = wait, "rate-index scheduler: next import");
        tokio::time::sleep(Duration::from_secs(wait)).await;
        match import(&pool, &actor, &settings).await {
            Ok(out) => tracing::info!(
                sheets = out.outcome.sheets.len(),
                publication = ?out.publication.as_ref().map(|p| p.id),
                "scheduled rate-index import complete"
            ),
            Err(e) => tracing::error!(error = %e, "scheduled rate-index import failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_spec_is_valid_and_at_par() {
        let spec = sheet_spec(7.29);
        let parsed = crate::rules::rate_sheet::RateSheetSpec::parse(&spec).unwrap();
        assert_eq!(parsed.rates, vec![7.29]);
        assert_eq!(parsed.lock_terms, vec![30]);
        assert_eq!(parsed.price(7.29, 30), Some(100.0));
    }

    #[test]
    fn daily_at_parses_and_rejects() {
        assert_eq!(
            parse_daily_at("14:30").unwrap(),
            NaiveTime::from_hms_opt(14, 30, 0).unwrap()
        );
        assert!(parse_daily_at("25:00").is_err());
        assert!(parse_daily_at("2pm").is_err());
    }

    #[test]
    fn secs_until_daily_wraps_midnight() {
        let at = NaiveTime::from_hms_opt(14, 0, 0).unwrap();
        let before = DateTime::parse_from_rfc3339("2026-01-01T13:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(secs_until_daily(before, at), 3600);
        let after = DateTime::parse_from_rfc3339("2026-01-01T14:00:01Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(secs_until_daily(after, at), 86399);
    }
}
