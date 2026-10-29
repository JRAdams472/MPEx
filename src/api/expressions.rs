use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::AppState;
use crate::error::AppError;

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
    let expressions = match super::parse_admin_as_of(q.as_of.as_deref())? {
        None => {
            let rows = sqlx::query_as::<_, (String, String, String, DateTime<Utc>, bool)>(
                "SELECT e.key, e.kind, e.description, e.updated_at,
                        EXISTS (SELECT 1 FROM expressions p
                                WHERE p.key = e.key AND p.status = 'published')
                 FROM expressions e WHERE e.status = 'draft' ORDER BY e.key",
            )
            .fetch_all(&state.pool)
            .await?;
            rows.into_iter()
                .map(|(key, kind, description, updated_at, has_published)| {
                    json!({
                        "key": key,
                        "kind": kind,
                        "description": description,
                        "status": "draft",
                        "updatedAt": updated_at,
                        "hasPublished": has_published,
                    })
                })
                .collect::<Vec<_>>()
        }
        Some(ts) => {
            let rows =
                sqlx::query_as::<_, (String, String, String, Option<DateTime<Utc>>, Option<Uuid>)>(
                    "SELECT DISTINCT ON (key) key, kind, description, published_at, publication_id
                 FROM expressions
                 WHERE status = 'published' AND published_at <= $1
                 ORDER BY key, published_at DESC",
                )
                .bind(ts)
                .fetch_all(&state.pool)
                .await?;
            rows.into_iter()
                .map(|(key, kind, description, published_at, publication_id)| {
                    json!({
                        "key": key,
                        "kind": kind,
                        "description": description,
                        "status": "published",
                        "publishedAt": published_at,
                        "publicationId": publication_id,
                    })
                })
                .collect()
        }
    };
    Ok(Json(json!({ "expressions": expressions })))
}

type ExpressionRow = (
    Uuid,
    String,
    String,
    String,
    String,
    Option<DateTime<Utc>>,
    Option<Uuid>,
    String,
    DateTime<Utc>,
    DateTime<Utc>,
);

const ROW_COLS: &str = "id, key, kind, description, status, published_at, publication_id, expression, created_at, updated_at";

fn row_json(
    (
        id,
        key,
        kind,
        description,
        status,
        published_at,
        publication_id,
        expression,
        created_at,
        updated_at,
    ): ExpressionRow,
) -> Value {
    json!({
        "id": id,
        "key": key,
        "kind": kind,
        "description": description,
        "status": status,
        "publishedAt": published_at,
        "publicationId": publication_id,
        "expression": expression,
        "createdAt": created_at,
        "updatedAt": updated_at,
    })
}

pub async fn get(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<Value>, AppError> {
    let draft = sqlx::query_as::<_, ExpressionRow>(&format!(
        "SELECT {ROW_COLS} FROM expressions WHERE key = $1 AND status = 'draft'"
    ))
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?;
    let published = sqlx::query_as::<_, ExpressionRow>(&format!(
        "SELECT {ROW_COLS} FROM expressions
             WHERE key = $1 AND status = 'published'
             ORDER BY published_at DESC LIMIT 1"
    ))
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?;
    if draft.is_none() && published.is_none() {
        return Err(AppError::NotFound(format!("expression {key}")));
    }

    let history = sqlx::query_as::<_, (Option<DateTime<Utc>>, Option<Uuid>, Option<String>)>(
        "SELECT e.published_at, e.publication_id, p.label
         FROM expressions e LEFT JOIN publications p ON p.id = e.publication_id
         WHERE e.key = $1 AND e.status = 'published'
         ORDER BY e.published_at DESC",
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
#[serde(rename_all = "camelCase")]
pub struct UpsertExpression {
    /// "standard" (default) or "unary" — decides which ZEN compiler validates.
    pub kind: Option<String>,
    pub description: Option<String>,
    pub expression: String,
}

/// Upsert the mutable draft for `key`; the expression is compile-checked by
/// the matching ZEN parser before it's stored.
pub async fn put(
    State(state): State<AppState>,
    principal: crate::auth::Principal,
    Path(key): Path<String>,
    Json(req): Json<UpsertExpression>,
) -> Result<Json<Value>, AppError> {
    if req.expression.trim().is_empty() {
        return Err(AppError::BadRequest("expression must not be empty".into()));
    }
    let kind = req.kind.unwrap_or_else(|| "standard".into());
    let compiled = match kind.as_str() {
        "standard" => zen_expression::compile_expression(&req.expression)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "unary" => zen_expression::compile_unary_expression(&req.expression)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        other => {
            return Err(AppError::BadRequest(format!(
                "kind must be 'standard' or 'unary', got {other:?}"
            )))
        }
    };
    if let Err(e) = compiled {
        return Err(AppError::BadRequest(format!("invalid expression: {e}")));
    }

    let mut tx = state.pool.begin().await?;
    let row = sqlx::query_as::<_, ExpressionRow>(&format!(
        "INSERT INTO expressions (key, kind, description, status, expression)
             VALUES ($1, $2, $3, 'draft', $4)
             ON CONFLICT (key) WHERE status = 'draft'
             DO UPDATE SET kind = EXCLUDED.kind,
                           description = EXCLUDED.description,
                           expression = EXCLUDED.expression,
                           updated_at = now()
             RETURNING {ROW_COLS}"
    ))
    .bind(&key)
    .bind(&kind)
    .bind(req.description.unwrap_or_default())
    .bind(&req.expression)
    .fetch_one(&mut *tx)
    .await?;
    crate::audit::record(
        &mut tx,
        &principal,
        crate::audit::Event {
            action: crate::audit::action::ARTIFACT_PUT,
            artifact_table: Some("expressions"),
            artifact_key: Some(key.clone()),
            detail: json!({
                "kind": kind,
                "contentHash": crate::audit::content_hash_str(&req.expression),
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
    let mut tx = state.pool.begin().await?;
    let n = sqlx::query("DELETE FROM expressions WHERE key = $1 AND status = 'draft'")
        .bind(&key)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound(format!("draft expression {key}")));
    }
    crate::audit::record(
        &mut tx,
        &principal,
        crate::audit::Event {
            action: crate::audit::action::ARTIFACT_DELETE,
            artifact_table: Some("expressions"),
            artifact_key: Some(key),
            detail: json!({}),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
