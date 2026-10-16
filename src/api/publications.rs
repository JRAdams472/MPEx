use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::AppState;
use crate::error::AppError;
use crate::rules::publish;

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let rows = sqlx::query_as::<_, (Uuid, String, String, DateTime<Utc>, i64, i64, i64, i64)>(
        "SELECT p.id, p.label, p.status, p.published_at,
                (SELECT count(*) FROM decisions d WHERE d.publication_id = p.id),
                (SELECT count(*) FROM expressions e WHERE e.publication_id = p.id),
                (SELECT count(*) FROM matrices m WHERE m.publication_id = p.id),
                (SELECT count(*) FROM rate_sheets s WHERE s.publication_id = p.id)
         FROM publications p ORDER BY p.published_at DESC",
    )
    .fetch_all(&state.pool)
    .await?;
    let publications: Vec<Value> = rows
        .into_iter()
        .map(
            |(
                id,
                label,
                status,
                published_at,
                decision_count,
                expression_count,
                matrix_count,
                rate_sheet_count,
            )| {
                json!({
                    "id": id,
                    "label": label,
                    "status": status,
                    "publishedAt": published_at,
                    "decisionCount": decision_count,
                    "expressionCount": expression_count,
                    "matrixCount": matrix_count,
                    "rateSheetCount": rate_sheet_count,
                })
            },
        )
        .collect();
    Ok(Json(json!({ "publications": publications })))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let row = sqlx::query_as::<_, (Uuid, String, String, DateTime<Utc>)>(
        "SELECT id, label, status, published_at FROM publications WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((id, label, status, published_at)) = row else {
        return Err(AppError::NotFound(format!("publication {id}")));
    };

    let decisions: Vec<String> =
        sqlx::query_scalar("SELECT key FROM decisions WHERE publication_id = $1 ORDER BY key")
            .bind(id)
            .fetch_all(&state.pool)
            .await?;
    let expressions: Vec<String> =
        sqlx::query_scalar("SELECT key FROM expressions WHERE publication_id = $1 ORDER BY key")
            .bind(id)
            .fetch_all(&state.pool)
            .await?;
    let matrices: Vec<String> =
        sqlx::query_scalar("SELECT key FROM matrices WHERE publication_id = $1 ORDER BY key")
            .bind(id)
            .fetch_all(&state.pool)
            .await?;
    let rate_sheets: Vec<String> =
        sqlx::query_scalar("SELECT key FROM rate_sheets WHERE publication_id = $1 ORDER BY key")
            .bind(id)
            .fetch_all(&state.pool)
            .await?;

    // The stamped key set is also the diff vs the prior publication — publish
    // only stamps drafts whose content changed.
    Ok(Json(json!({
        "id": id,
        "label": label,
        "status": status,
        "publishedAt": published_at,
        "decisions": decisions,
        "expressions": expressions,
        "matrices": matrices,
        "rateSheets": rate_sheets,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishRequest {
    pub label: Option<String>,
}

/// Atomically publishes every changed draft artifact (decisions + expressions)
/// under one named publication. 409 when nothing changed since the last
/// publish — empty publications are never stamped.
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<PublishRequest>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let label = req.label.unwrap_or_default();
    let publication = publish::publish(&state.pool, &label).await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(publication).unwrap_or_default()),
    ))
}
