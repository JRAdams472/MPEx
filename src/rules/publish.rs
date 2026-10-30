use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;

/// Result of one atomic publish: the publication row plus the artifact keys it
/// stamped. Only drafts whose content differs from their latest published row
/// (or that were never published) get a new published row — history never
/// duplicates unchanged documents.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Publication {
    pub id: Uuid,
    pub label: String,
    pub status: String,
    pub published_at: DateTime<Utc>,
    /// Lane this publication stamped: 'dev' for publishes, the target lane
    /// for promotions.
    pub env: String,
    pub decisions: Vec<String>,
    pub expressions: Vec<String>,
    pub matrices: Vec<String>,
    pub rate_sheets: Vec<String>,
}

/// Drafts become immutable published rows in a single transaction: insert the
/// publication, copy each changed draft with `published_at` set to the
/// publication timestamp, commit. New rows bump `max(updated_at)`, which
/// naturally invalidates the snapshot fingerprint cache — no explicit cache
/// eviction needed.
#[tracing::instrument(
    name = "publish.transaction",
    skip(pool),
    fields(
        id = tracing::field::Empty,
        decisions = tracing::field::Empty,
        expressions = tracing::field::Empty,
        matrices = tracing::field::Empty,
        rate_sheets = tracing::field::Empty
    )
)]
pub async fn publish(
    pool: &PgPool,
    label: &str,
    actor: crate::audit::Actor,
) -> Result<Publication, AppError> {
    let label = label.trim();
    if label.is_empty() {
        return Err(AppError::BadRequest("publication label is required".into()));
    }
    crate::api::validate::validate_label(label)?;

    let mut tx = pool.begin().await?;
    let (id, published_at): (Uuid, DateTime<Utc>) =
        sqlx::query_as("INSERT INTO publications (label) VALUES ($1) RETURNING id, published_at")
            .bind(label)
            .fetch_one(&mut *tx)
            .await?;

    let decisions: Vec<String> = sqlx::query_scalar(
        "WITH latest AS (
           SELECT DISTINCT ON (key) key, kind, product_code, jdm
           FROM decisions WHERE status = 'published' AND env = 'dev'
           ORDER BY key, published_at DESC
         ), ins AS (
           INSERT INTO decisions
             (key, kind, product_code, status, published_at, publication_id, jdm, env)
           SELECT d.key, d.kind, d.product_code, 'published', $1, $2, d.jdm, d.env
           FROM decisions d
           LEFT JOIN latest p ON p.key = d.key
           WHERE d.status = 'draft' AND d.env = 'dev'
             AND (p.key IS NULL
                  OR p.jdm IS DISTINCT FROM d.jdm
                  OR p.kind IS DISTINCT FROM d.kind
                  OR p.product_code IS DISTINCT FROM d.product_code)
           RETURNING key
         )
         SELECT key FROM ins ORDER BY key",
    )
    .bind(published_at)
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;

    let expressions: Vec<String> = sqlx::query_scalar(
        "WITH latest AS (
           SELECT DISTINCT ON (key) key, kind, description, expression
           FROM expressions WHERE status = 'published' AND env = 'dev'
           ORDER BY key, published_at DESC
         ), ins AS (
           INSERT INTO expressions
             (key, kind, description, status, published_at, publication_id, expression, env)
           SELECT d.key, d.kind, d.description, 'published', $1, $2, d.expression, d.env
           FROM expressions d
           LEFT JOIN latest p ON p.key = d.key
           WHERE d.status = 'draft' AND d.env = 'dev'
             AND (p.key IS NULL
                  OR p.expression IS DISTINCT FROM d.expression
                  OR p.kind IS DISTINCT FROM d.kind
                  OR p.description IS DISTINCT FROM d.description)
           RETURNING key
         )
         SELECT key FROM ins ORDER BY key",
    )
    .bind(published_at)
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;

    // Matrices: `spec` is the source of truth (`jdm` is derived at write
    // time), so change detection compares spec/kind/product_code.
    let matrices: Vec<String> = sqlx::query_scalar(
        "WITH latest AS (
           SELECT DISTINCT ON (key) key, kind, product_code, spec
           FROM matrices WHERE status = 'published' AND env = 'dev'
           ORDER BY key, published_at DESC
         ), ins AS (
           INSERT INTO matrices
             (key, kind, product_code, status, published_at, publication_id, spec, jdm, env)
           SELECT d.key, d.kind, d.product_code, 'published', $1, $2, d.spec, d.jdm, d.env
           FROM matrices d
           LEFT JOIN latest p ON p.key = d.key
           WHERE d.status = 'draft' AND d.env = 'dev'
             AND (p.key IS NULL
                  OR p.spec IS DISTINCT FROM d.spec
                  OR p.kind IS DISTINCT FROM d.kind
                  OR p.product_code IS DISTINCT FROM d.product_code)
           RETURNING key
         )
         SELECT key FROM ins ORDER BY key",
    )
    .bind(published_at)
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;

    // Rate sheets: `spec` is the whole artifact (a data grid, not rules).
    let rate_sheets = changed_rate_sheet_drafts(&mut tx).await?;
    stamp_rate_sheet_drafts(&mut tx, published_at, id, &rate_sheets).await?;

    if decisions.is_empty()
        && expressions.is_empty()
        && matrices.is_empty()
        && rate_sheets.is_empty()
    {
        // Rolling back drops the publication row too — no empty snapshots.
        return Err(AppError::Conflict("no draft changes to publish".into()));
    }

    // Inside the same tx: a rollback (e.g. the empty-publish 409 above) leaves
    // no audit row, and a committed publication is always attributed.
    crate::audit::record(
        &mut tx,
        actor,
        crate::audit::Event {
            action: crate::audit::action::PUBLICATION_CREATE,
            artifact_table: Some("publications"),
            artifact_key: Some(id.to_string()),
            detail: serde_json::json!({
                "label": label,
                "decisionCount": decisions.len(),
                "expressionCount": expressions.len(),
                "matrixCount": matrices.len(),
                "rateSheetCount": rate_sheets.len(),
            }),
        },
    )
    .await?;

    tx.commit().await?;
    tracing::Span::current()
        .record("id", tracing::field::display(id))
        .record("decisions", decisions.len())
        .record("expressions", expressions.len())
        .record("matrices", matrices.len())
        .record("rate_sheets", rate_sheets.len());
    tracing::info!(
        publication = %id,
        decisions = decisions.len(),
        expressions = expressions.len(),
        matrices = matrices.len(),
        rate_sheets = rate_sheets.len(),
        "published"
    );
    Ok(Publication {
        id,
        label: label.to_string(),
        status: "published".into(),
        published_at,
        env: "dev".into(),
        decisions,
        expressions,
        matrices,
        rate_sheets,
    })
}

/// Rate-sheet draft keys whose content differs from their latest published
/// row (or that were never published) — the set `publish` stamps.
async fn changed_rate_sheet_drafts(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<Vec<String>, AppError> {
    sqlx::query_scalar(
        "WITH latest AS (
           SELECT DISTINCT ON (key) key, spec
           FROM rate_sheets WHERE status = 'published' AND env = 'dev'
           ORDER BY key, published_at DESC
         )
         SELECT d.key FROM rate_sheets d
           LEFT JOIN latest p ON p.key = d.key
         WHERE d.status = 'draft' AND d.env = 'dev'
           AND (p.key IS NULL OR p.spec IS DISTINCT FROM d.spec)
         ORDER BY d.key",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(AppError::from)
}

/// Copies each listed draft into an immutable published row under `id`.
async fn stamp_rate_sheet_drafts(
    tx: &mut Transaction<'_, Postgres>,
    published_at: DateTime<Utc>,
    publication_id: Uuid,
    keys: &[String],
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO rate_sheets (key, status, published_at, publication_id, spec, env)
         SELECT d.key, 'published', $1, $2, d.spec, d.env
         FROM rate_sheets d
         WHERE d.status = 'draft' AND d.env = 'dev' AND d.key = ANY($3)",
    )
    .bind(published_at)
    .bind(publication_id)
    .bind(keys)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Publishes just the rate sheets, inside the caller's transaction — used
/// by the rate-index importer so an unattended import never sweeps
/// unrelated decision/matrix/expression drafts into a publication.
/// Returns `None` when no sheet changed (the import is still recorded;
/// no empty publication is stamped).
pub async fn publish_rate_sheets(
    tx: &mut Transaction<'_, Postgres>,
    label: &str,
    actor: crate::audit::Actor,
) -> Result<Option<Publication>, AppError> {
    let label = label.trim();
    if label.is_empty() {
        return Err(AppError::BadRequest("publication label is required".into()));
    }
    crate::api::validate::validate_label(label)?;

    let rate_sheets = changed_rate_sheet_drafts(tx).await?;
    if rate_sheets.is_empty() {
        return Ok(None);
    }
    let (id, published_at): (Uuid, DateTime<Utc>) =
        sqlx::query_as("INSERT INTO publications (label) VALUES ($1) RETURNING id, published_at")
            .bind(label)
            .fetch_one(&mut **tx)
            .await?;
    stamp_rate_sheet_drafts(tx, published_at, id, &rate_sheets).await?;
    crate::audit::record(
        tx,
        actor,
        crate::audit::Event {
            action: crate::audit::action::PUBLICATION_CREATE,
            artifact_table: Some("publications"),
            artifact_key: Some(id.to_string()),
            detail: serde_json::json!({
                "label": label,
                "decisionCount": 0,
                "expressionCount": 0,
                "matrixCount": 0,
                "rateSheetCount": rate_sheets.len(),
            }),
        },
    )
    .await?;
    Ok(Some(Publication {
        id,
        label: label.to_string(),
        status: "published".into(),
        published_at,
        env: "dev".into(),
        decisions: vec![],
        expressions: vec![],
        matrices: vec![],
        rate_sheets,
    }))
}

/// Promote one lane forward (MPE-153): copies the latest published row per
/// key from `from` into `to` where it differs from `to`'s latest — the same
/// only-changed semantics `publish` uses against drafts. Adjacency is
/// enforced (`dev→qa→uat→prod`, no skips or reversals), the whole stamp is
/// one transaction, and an empty delta is a 409 like an empty publish.
#[tracing::instrument(name = "promote.transaction", skip(pool))]
pub async fn promote(
    pool: &PgPool,
    from: crate::rules::loader::Env,
    to: crate::rules::loader::Env,
    label: &str,
    actor: crate::audit::Actor,
) -> Result<Publication, AppError> {
    if from.next() != Some(to) {
        return Err(AppError::BadRequest(format!(
            "promotions advance one lane at a time (dev→qa→uat→prod); \
             {}→{} is not allowed",
            from.as_str(),
            to.as_str()
        )));
    }
    let label = label.trim();
    if label.is_empty() {
        return Err(AppError::BadRequest("promotion label is required".into()));
    }
    crate::api::validate::validate_label(label)?;

    let mut tx = pool.begin().await?;
    let (id, published_at): (Uuid, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO publications (label, env) VALUES ($1, $2) RETURNING id, published_at",
    )
    .bind(label)
    .bind(to.as_str())
    .fetch_one(&mut *tx)
    .await?;

    // Per table: src = latest published per key in `from`; dst = latest in
    // `to`; stamp src rows whose content differs (or that `to` lacks).
    let decisions: Vec<String> = sqlx::query_scalar(
        "WITH src AS (
           SELECT DISTINCT ON (key) key, kind, product_code, jdm FROM decisions
           WHERE status = 'published' AND env = $3 ORDER BY key, published_at DESC
         ), dst AS (
           SELECT DISTINCT ON (key) key, kind, product_code, jdm FROM decisions
           WHERE status = 'published' AND env = $4 ORDER BY key, published_at DESC
         ), ins AS (
           INSERT INTO decisions
             (key, kind, product_code, status, published_at, publication_id, jdm, env)
           SELECT s.key, s.kind, s.product_code, 'published', $1, $2, s.jdm, $4
           FROM src s LEFT JOIN dst d ON d.key = s.key
           WHERE d.key IS NULL
              OR d.jdm IS DISTINCT FROM s.jdm
              OR d.kind IS DISTINCT FROM s.kind
              OR d.product_code IS DISTINCT FROM s.product_code
           RETURNING key
         ) SELECT key FROM ins ORDER BY key",
    )
    .bind(published_at)
    .bind(id)
    .bind(from.as_str())
    .bind(to.as_str())
    .fetch_all(&mut *tx)
    .await?;

    let expressions: Vec<String> = sqlx::query_scalar(
        "WITH src AS (
           SELECT DISTINCT ON (key) key, kind, description, expression FROM expressions
           WHERE status = 'published' AND env = $3 ORDER BY key, published_at DESC
         ), dst AS (
           SELECT DISTINCT ON (key) key, kind, description, expression FROM expressions
           WHERE status = 'published' AND env = $4 ORDER BY key, published_at DESC
         ), ins AS (
           INSERT INTO expressions
             (key, kind, description, status, published_at, publication_id, expression, env)
           SELECT s.key, s.kind, s.description, 'published', $1, $2, s.expression, $4
           FROM src s LEFT JOIN dst d ON d.key = s.key
           WHERE d.key IS NULL
              OR d.expression IS DISTINCT FROM s.expression
              OR d.kind IS DISTINCT FROM s.kind
              OR d.description IS DISTINCT FROM s.description
           RETURNING key
         ) SELECT key FROM ins ORDER BY key",
    )
    .bind(published_at)
    .bind(id)
    .bind(from.as_str())
    .bind(to.as_str())
    .fetch_all(&mut *tx)
    .await?;

    let matrices: Vec<String> = sqlx::query_scalar(
        "WITH src AS (
           SELECT DISTINCT ON (key) key, kind, product_code, spec, jdm FROM matrices
           WHERE status = 'published' AND env = $3 ORDER BY key, published_at DESC
         ), dst AS (
           SELECT DISTINCT ON (key) key, kind, product_code, spec, jdm FROM matrices
           WHERE status = 'published' AND env = $4 ORDER BY key, published_at DESC
         ), ins AS (
           INSERT INTO matrices
             (key, kind, product_code, status, published_at, publication_id, spec, jdm, env)
           SELECT s.key, s.kind, s.product_code, 'published', $1, $2, s.spec, s.jdm, $4
           FROM src s LEFT JOIN dst d ON d.key = s.key
           WHERE d.key IS NULL
              OR d.spec IS DISTINCT FROM s.spec
              OR d.jdm IS DISTINCT FROM s.jdm
              OR d.kind IS DISTINCT FROM s.kind
              OR d.product_code IS DISTINCT FROM s.product_code
           RETURNING key
         ) SELECT key FROM ins ORDER BY key",
    )
    .bind(published_at)
    .bind(id)
    .bind(from.as_str())
    .bind(to.as_str())
    .fetch_all(&mut *tx)
    .await?;

    let rate_sheets: Vec<String> = sqlx::query_scalar(
        "WITH src AS (
           SELECT DISTINCT ON (key) key, spec FROM rate_sheets
           WHERE status = 'published' AND env = $3 ORDER BY key, published_at DESC
         ), dst AS (
           SELECT DISTINCT ON (key) key, spec FROM rate_sheets
           WHERE status = 'published' AND env = $4 ORDER BY key, published_at DESC
         ), ins AS (
           INSERT INTO rate_sheets
             (key, status, published_at, publication_id, spec, env)
           SELECT s.key, 'published', $1, $2, s.spec, $4
           FROM src s LEFT JOIN dst d ON d.key = s.key
           WHERE d.key IS NULL OR d.spec IS DISTINCT FROM s.spec
           RETURNING key
         ) SELECT key FROM ins ORDER BY key",
    )
    .bind(published_at)
    .bind(id)
    .bind(from.as_str())
    .bind(to.as_str())
    .fetch_all(&mut *tx)
    .await?;

    if decisions.is_empty()
        && expressions.is_empty()
        && matrices.is_empty()
        && rate_sheets.is_empty()
    {
        return Err(AppError::Conflict(format!(
            "nothing to promote: {0} and {1} already agree on every artifact",
            from.as_str(),
            to.as_str()
        )));
    }

    crate::audit::record(
        &mut tx,
        actor,
        crate::audit::Event {
            action: crate::audit::action::PROMOTION_CREATE,
            artifact_table: Some("publications"),
            artifact_key: Some(id.to_string()),
            detail: serde_json::json!({
                "label": label,
                "from": from.as_str(),
                "to": to.as_str(),
                "decisionCount": decisions.len(),
                "expressionCount": expressions.len(),
                "matrixCount": matrices.len(),
                "rateSheetCount": rate_sheets.len(),
            }),
        },
    )
    .await?;

    tx.commit().await?;
    tracing::info!(
        publication = %id, from = from.as_str(), to = to.as_str(),
        decisions = decisions.len(), expressions = expressions.len(),
        matrices = matrices.len(), rate_sheets = rate_sheets.len(),
        "promoted"
    );
    Ok(Publication {
        id,
        label: label.to_string(),
        status: "published".into(),
        published_at,
        env: to.as_str().to_string(),
        decisions,
        expressions,
        matrices,
        rate_sheets,
    })
}
