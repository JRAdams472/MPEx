use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::AppState;
use crate::audit::{self, Actor};
use crate::error::AppError;
use crate::rate_index::{self, IndexSettings};

fn settings(state: &AppState) -> Result<&IndexSettings, AppError> {
    state
        .rate_index
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("rate index is not configured (RATE_INDEX_URL)".into()))
}

/// GET /api/rate-index — the tracked index labels plus recent imports
/// (viewer+, alongside the other admin GETs).
pub async fn get(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let imports = sqlx::query_as::<
        _,
        (
            Uuid,
            DateTime<Utc>,
            String,
            String,
            Value,
            String,
            Option<Uuid>,
            DateTime<Utc>,
        ),
    >(
        "SELECT id, fetched_at, source_url, content_hash, rates, imported_by,
                publication_id, created_at
         FROM rate_index_imports ORDER BY created_at DESC LIMIT 20",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({
        "indexLabels": rate_index::parser::INDEX_LABELS,
        "imports": imports
            .into_iter()
            .map(|(id, fetched_at, url, hash, rates, by, pub_id, created)| {
                json!({
                    "id": id,
                    "fetchedAt": fetched_at,
                    "sourceUrl": url,
                    "contentHash": hash,
                    "rates": rates,
                    "importedBy": by,
                    "publicationId": pub_id,
                    "createdAt": created,
                })
            })
            .collect::<Vec<_>>(),
    })))
}

/// POST /api/admin/rate-index/preview — fetch + parse + resolve against
/// the catalog; writes nothing.
pub async fn preview(
    State(state): State<AppState>,
    _principal: crate::auth::Principal,
) -> Result<Json<Value>, AppError> {
    let settings = settings(&state)?;
    let fetched = rate_index::fetch_index(settings).await?;
    let rates = rate_index::parser::parse_index(&fetched.html).map_err(AppError::BadGateway)?;
    let outcome = rate_index::resolve(&state.pool, &rates).await?;
    Ok(Json(json!({
        "fetchedAt": fetched.fetched_at,
        "sourceUrl": settings.url,
        "contentHash": fetched.content_hash,
        "indexRates": rate_index::parser::INDEX_LABELS
            .iter()
            .map(|l| json!({ "label": l, "rate": rates[*l] }))
            .collect::<Vec<_>>(),
        "sheets": outcome.sheets,
        "skipped": outcome.skipped,
        "unmapped": outcome.unmapped,
    })))
}

/// POST /api/admin/rate-index/import — fetch, write drafts, publish.
/// Atomic: a fetch/parse failure or an empty resolution writes nothing.
pub async fn import(
    State(state): State<AppState>,
    principal: crate::auth::Principal,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let settings = settings(&state)?.clone();
    let out = rate_index::import(&state.pool, &Actor::from(&principal), &settings).await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(out).unwrap_or_default()),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RateConfigRequest {
    /// MND index label — makes this a base product. Null clears it.
    pub mnd_label: Option<String>,
    /// Base product code — makes this a derived product. Null clears it.
    pub base_product_code: Option<String>,
    /// Points added to the base index rate (derived products only).
    #[serde(default)]
    pub rate_adjustment: f64,
}

/// PUT /api/admin/products/{code}/rate-config — set a product's rate
/// source: index label (base), base product + adjustment (derived), or
/// neither (manual sheet). Full replace; audited.
pub async fn put_rate_config(
    State(state): State<AppState>,
    principal: crate::auth::Principal,
    Path(code): Path<String>,
    Json(req): Json<RateConfigRequest>,
) -> Result<Json<Value>, AppError> {
    super::validate::validate_key(&code)?;
    if req.mnd_label.is_some() && req.base_product_code.is_some() {
        return Err(AppError::BadRequest(
            "set mndLabel or baseProductCode, not both".into(),
        ));
    }
    if let Some(label) = &req.mnd_label {
        if !rate_index::parser::INDEX_LABELS.contains(&label.as_str()) {
            return Err(AppError::BadRequest(format!(
                "mndLabel must be one of {:?}",
                rate_index::parser::INDEX_LABELS
            )));
        }
    }
    if !req.rate_adjustment.is_finite() || req.rate_adjustment.abs() > 5.0 {
        return Err(AppError::BadRequest(
            "rateAdjustment must be finite and within +/-5 points".into(),
        ));
    }
    if req.base_product_code.is_none() && req.rate_adjustment != 0.0 {
        return Err(AppError::BadRequest(
            "rateAdjustment requires baseProductCode".into(),
        ));
    }
    if req.base_product_code.as_deref() == Some(code.as_str()) {
        return Err(AppError::BadRequest(
            "a product cannot derive from itself".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM products WHERE code = $1)")
        .bind(&code)
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        return Err(AppError::NotFound(format!("product {code}")));
    }
    if let Some(base) = &req.base_product_code {
        let base_is_index: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM products WHERE code = $1 AND mnd_label IS NOT NULL)",
        )
        .bind(base)
        .fetch_one(&mut *tx)
        .await?;
        if !base_is_index {
            return Err(AppError::BadRequest(format!(
                "baseProductCode {base} must be an index product (mndLabel set)"
            )));
        }
    }
    sqlx::query(
        "UPDATE products
         SET mnd_label = $2, base_product_code = $3, rate_adjustment = $4,
             updated_at = now()
         WHERE code = $1",
    )
    .bind(&code)
    .bind(&req.mnd_label)
    .bind(&req.base_product_code)
    .bind(req.rate_adjustment)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        &principal,
        audit::Event {
            action: audit::action::PRODUCT_RATE_CONFIG_PUT,
            artifact_table: Some("products"),
            artifact_key: Some(code.clone()),
            detail: json!({
                "mndLabel": req.mnd_label,
                "baseProductCode": req.base_product_code,
                "rateAdjustment": req.rate_adjustment,
            }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({
        "code": code,
        "mndLabel": req.mnd_label,
        "baseProductCode": req.base_product_code,
        "rateAdjustment": req.rate_adjustment,
    })))
}
