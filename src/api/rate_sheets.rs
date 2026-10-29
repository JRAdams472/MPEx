use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::AppState;
use crate::error::AppError;
use crate::rules::rate_sheet::RateSheetSpec;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    /// Omitted or "draft" = working drafts; RFC3339 = the published snapshot
    /// as of that timestamp.
    pub as_of: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    let sheets = match super::parse_admin_as_of(q.as_of.as_deref())? {
        None => {
            let rows = sqlx::query_as::<_, (String, DateTime<Utc>, bool)>(
                "SELECT s.key, s.updated_at,
                        EXISTS (SELECT 1 FROM rate_sheets p
                                WHERE p.key = s.key AND p.status = 'published')
                 FROM rate_sheets s WHERE s.status = 'draft' ORDER BY s.key",
            )
            .fetch_all(&state.pool)
            .await?;
            rows.into_iter()
                .map(|(key, updated_at, has_published)| {
                    json!({
                        "key": key,
                        "status": "draft",
                        "updatedAt": updated_at,
                        "hasPublished": has_published,
                    })
                })
                .collect::<Vec<_>>()
        }
        Some(ts) => {
            let rows = sqlx::query_as::<_, (String, Option<DateTime<Utc>>, Option<Uuid>)>(
                "SELECT DISTINCT ON (key) key, published_at, publication_id
                 FROM rate_sheets
                 WHERE status = 'published' AND published_at <= $1
                 ORDER BY key, published_at DESC",
            )
            .bind(ts)
            .fetch_all(&state.pool)
            .await?;
            rows.into_iter()
                .map(|(key, published_at, publication_id)| {
                    json!({
                        "key": key,
                        "status": "published",
                        "publishedAt": published_at,
                        "publicationId": publication_id,
                    })
                })
                .collect()
        }
    };
    Ok(Json(json!({ "rateSheets": sheets })))
}

type SheetRow = (
    Uuid,
    String,
    String,
    Option<DateTime<Utc>>,
    Option<Uuid>,
    Value,
    DateTime<Utc>,
    DateTime<Utc>,
);

const ROW_COLS: &str =
    "id, key, status, published_at, publication_id, spec, created_at, updated_at";

fn row_json(
    (id, key, status, published_at, publication_id, spec, created_at, updated_at): SheetRow,
) -> Value {
    json!({
        "id": id,
        "key": key,
        "status": status,
        "publishedAt": published_at,
        "publicationId": publication_id,
        "spec": spec,
        "createdAt": created_at,
        "updatedAt": updated_at,
    })
}

pub async fn get(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<Value>, AppError> {
    super::validate::validate_key(&key)?;
    let draft = sqlx::query_as::<_, SheetRow>(&format!(
        "SELECT {ROW_COLS} FROM rate_sheets WHERE key = $1 AND status = 'draft'"
    ))
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?;
    let published = sqlx::query_as::<_, SheetRow>(&format!(
        "SELECT {ROW_COLS} FROM rate_sheets
             WHERE key = $1 AND status = 'published'
             ORDER BY published_at DESC LIMIT 1"
    ))
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?;
    if draft.is_none() && published.is_none() {
        return Err(AppError::NotFound(format!("rate sheet {key}")));
    }

    let history = sqlx::query_as::<_, (Option<DateTime<Utc>>, Option<Uuid>, Option<String>)>(
        "SELECT s.published_at, s.publication_id, p.label
         FROM rate_sheets s LEFT JOIN publications p ON p.id = s.publication_id
         WHERE s.key = $1 AND s.status = 'published'
         ORDER BY s.published_at DESC",
    )
    .bind(&key)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(published_at, publication_id, label)| {
        json!({
            "publishedAt": published_at,
            "publicationId": publication_id,
            "publicationLabel": label,
        })
    })
    .collect::<Vec<_>>();

    Ok(Json(json!({
        "key": key,
        "draft": draft.map(row_json),
        "published": published.map(row_json),
        "history": history,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpsertSheet {
    /// The full grid spec (PUT fully replaces the draft).
    pub spec: Value,
}

/// Upsert the mutable draft for `key` — which is the product code, so the
/// product must already exist.
pub async fn put(
    State(state): State<AppState>,
    principal: crate::auth::Principal,
    Path(key): Path<String>,
    Json(req): Json<UpsertSheet>,
) -> Result<Json<Value>, AppError> {
    super::validate::validate_key(&key)?;
    super::validate::validate_spec_size(&req.spec, "spec")?;
    RateSheetSpec::parse(&req.spec).map_err(AppError::BadRequest)?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM products WHERE code = $1)")
        .bind(&key)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::BadRequest(format!(
            "rate sheet key must be a known product code, got {key}"
        )));
    }

    let mut tx = state.pool.begin().await?;
    let row = sqlx::query_as::<_, SheetRow>(&format!(
        "INSERT INTO rate_sheets (key, status, spec)
             VALUES ($1, 'draft', $2)
             ON CONFLICT (key) WHERE status = 'draft'
             DO UPDATE SET spec = EXCLUDED.spec, updated_at = now()
             RETURNING {ROW_COLS}"
    ))
    .bind(&key)
    .bind(&req.spec)
    .fetch_one(&mut *tx)
    .await?;
    crate::audit::record(
        &mut tx,
        &principal,
        crate::audit::Event {
            action: crate::audit::action::ARTIFACT_PUT,
            artifact_table: Some("rate_sheets"),
            artifact_key: Some(key.clone()),
            detail: json!({
                "contentHash": crate::audit::content_hash(&req.spec),
            }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(row_json(row)))
}

/// Deletes the draft only; published history is immutable and stays.
pub async fn delete(
    State(state): State<AppState>,
    principal: crate::auth::Principal,
    Path(key): Path<String>,
) -> Result<StatusCode, AppError> {
    super::validate::validate_key(&key)?;
    let mut tx = state.pool.begin().await?;
    let n = sqlx::query("DELETE FROM rate_sheets WHERE key = $1 AND status = 'draft'")
        .bind(&key)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound(format!("draft rate sheet {key}")));
    }
    crate::audit::record(
        &mut tx,
        &principal,
        crate::audit::Event {
            action: crate::audit::action::ARTIFACT_DELETE,
            artifact_table: Some("rate_sheets"),
            artifact_key: Some(key),
            detail: json!({}),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
