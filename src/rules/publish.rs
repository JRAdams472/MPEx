use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
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
    pub decisions: Vec<String>,
    pub expressions: Vec<String>,
}

/// Drafts become immutable published rows in a single transaction: insert the
/// publication, copy each changed draft with `published_at` set to the
/// publication timestamp, commit. New rows bump `max(updated_at)`, which
/// naturally invalidates the snapshot fingerprint cache — no explicit cache
/// eviction needed.
pub async fn publish(pool: &PgPool, label: &str) -> Result<Publication, AppError> {
    let label = label.trim();
    if label.is_empty() {
        return Err(AppError::BadRequest("publication label is required".into()));
    }

    let mut tx = pool.begin().await?;
    let (id, published_at): (Uuid, DateTime<Utc>) =
        sqlx::query_as("INSERT INTO publications (label) VALUES ($1) RETURNING id, published_at")
            .bind(label)
            .fetch_one(&mut *tx)
            .await?;

    let decisions: Vec<String> = sqlx::query_scalar(
        "WITH latest AS (
           SELECT DISTINCT ON (key) key, kind, product_code, jdm
           FROM decisions WHERE status = 'published'
           ORDER BY key, published_at DESC
         ), ins AS (
           INSERT INTO decisions
             (key, kind, product_code, status, published_at, publication_id, jdm)
           SELECT d.key, d.kind, d.product_code, 'published', $1, $2, d.jdm
           FROM decisions d
           LEFT JOIN latest p ON p.key = d.key
           WHERE d.status = 'draft'
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
           FROM expressions WHERE status = 'published'
           ORDER BY key, published_at DESC
         ), ins AS (
           INSERT INTO expressions
             (key, kind, description, status, published_at, publication_id, expression)
           SELECT d.key, d.kind, d.description, 'published', $1, $2, d.expression
           FROM expressions d
           LEFT JOIN latest p ON p.key = d.key
           WHERE d.status = 'draft'
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

    if decisions.is_empty() && expressions.is_empty() {
        // Rolling back drops the publication row too — no empty snapshots.
        return Err(AppError::Conflict("no draft changes to publish".into()));
    }

    tx.commit().await?;
    tracing::info!(
        publication = %id,
        decisions = decisions.len(),
        expressions = expressions.len(),
        "published"
    );
    Ok(Publication {
        id,
        label: label.to_string(),
        status: "published".into(),
        published_at,
        decisions,
        expressions,
    })
}
