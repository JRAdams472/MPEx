use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::AppState;
use crate::error::AppError;
use crate::rules::matrix::{self, MatrixSpec};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    /// Omitted or "draft" = working drafts; RFC3339 = the published snapshot
    /// as of that timestamp.
    pub as_of: Option<String>,
    /// Lane the published (asOf) view resolves in; omitted = dev (MPE-153).
    pub env: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    let env = super::parse_env(q.env.as_deref())?;
    let matrices = match super::parse_admin_as_of(q.as_of.as_deref())? {
        None => {
            let rows = sqlx::query_as::<_, (String, String, Option<String>, DateTime<Utc>, bool)>(
                "SELECT m.key, m.kind, m.product_code, m.updated_at,
                        EXISTS (SELECT 1 FROM matrices p
                                WHERE p.key = m.key AND p.status = 'published'
                                  AND p.env = 'dev')
                 FROM matrices m WHERE m.status = 'draft' ORDER BY m.key",
            )
            .fetch_all(&state.pool)
            .await?;
            rows.into_iter()
                .map(|(key, kind, product_code, updated_at, has_published)| {
                    json!({
                        "key": key,
                        "kind": kind,
                        "productCode": product_code,
                        "status": "draft",
                        "updatedAt": updated_at,
                        "hasPublished": has_published,
                    })
                })
                .collect::<Vec<_>>()
        }
        Some(ts) => {
            let rows = sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    Option<String>,
                    Option<DateTime<Utc>>,
                    Option<Uuid>,
                ),
            >(
                "SELECT DISTINCT ON (key) key, kind, product_code, published_at, publication_id
                 FROM matrices
                 WHERE status = 'published' AND published_at <= $1 AND env = $2
                 ORDER BY key, published_at DESC",
            )
            .bind(ts)
            .bind(env.as_str())
            .fetch_all(&state.pool)
            .await?;
            rows.into_iter()
                .map(|(key, kind, product_code, published_at, publication_id)| {
                    json!({
                        "key": key,
                        "kind": kind,
                        "productCode": product_code,
                        "status": "published",
                        "publishedAt": published_at,
                        "publicationId": publication_id,
                    })
                })
                .collect()
        }
    };
    Ok(Json(json!({ "matrices": matrices })))
}

type MatrixRow = (
    Uuid,
    String,
    String,
    Option<String>,
    String,
    Option<DateTime<Utc>>,
    Option<Uuid>,
    Value,
    Value,
    DateTime<Utc>,
    DateTime<Utc>,
);

const ROW_COLS: &str = "id, key, kind, product_code, status, published_at, publication_id, spec, jdm, created_at, updated_at";

fn row_json(
    (
        id,
        key,
        kind,
        product_code,
        status,
        published_at,
        publication_id,
        spec,
        jdm,
        created_at,
        updated_at,
    ): MatrixRow,
) -> Value {
    json!({
        "id": id,
        "key": key,
        "kind": kind,
        "productCode": product_code,
        "status": status,
        "publishedAt": published_at,
        "publicationId": publication_id,
        "spec": spec,
        "jdm": jdm,
        "createdAt": created_at,
        "updatedAt": updated_at,
    })
}

pub async fn get(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<Value>, AppError> {
    super::validate::validate_key(&key)?;
    let draft = sqlx::query_as::<_, MatrixRow>(&format!(
        "SELECT {ROW_COLS} FROM matrices WHERE key = $1 AND status = 'draft'"
    ))
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?;
    let published = sqlx::query_as::<_, MatrixRow>(&format!(
        "SELECT {ROW_COLS} FROM matrices
             WHERE key = $1 AND status = 'published' AND env = 'dev'
             ORDER BY published_at DESC LIMIT 1"
    ))
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?;
    if draft.is_none() && published.is_none() {
        return Err(AppError::NotFound(format!("matrix {key}")));
    }

    let history =
        sqlx::query_as::<_, (Option<DateTime<Utc>>, Option<Uuid>, Option<String>, String)>(
            "SELECT m.published_at, m.publication_id, p.label, m.env
         FROM matrices m LEFT JOIN publications p ON p.id = m.publication_id
         WHERE m.key = $1 AND m.status = 'published'
         ORDER BY m.published_at DESC",
        )
        .bind(&key)
        .fetch_all(&state.pool)
        .await?
        .into_iter()
        .map(|(published_at, publication_id, label, env)| {
            json!({
                "publishedAt": published_at,
                "publicationId": publication_id,
                "publicationLabel": label,
                "env": env,
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
pub struct UpsertMatrix {
    /// Artifact kind; defaults to "llpa" (PUT fully replaces the draft).
    pub kind: Option<String>,
    pub product_code: Option<String>,
    pub spec: Value,
}

/// Upsert the mutable draft for `key`. The spec is validated and compiled to
/// JDM now, so drafts evaluate byte-for-byte like their future publications.
pub async fn put(
    State(state): State<AppState>,
    principal: crate::auth::Principal,
    Path(key): Path<String>,
    Json(req): Json<UpsertMatrix>,
) -> Result<Json<Value>, AppError> {
    super::validate::validate_key(&key)?;
    super::validate::validate_spec_size(&req.spec, "spec")?;
    if let Some(code) = &req.product_code {
        super::validate::validate_key(code)?;
    }
    let kind = req.kind.unwrap_or_else(|| "llpa".into());
    if !super::decisions::KINDS.contains(&kind.as_str()) {
        return Err(AppError::BadRequest(format!(
            "kind must be one of {:?}, got {kind:?}",
            super::decisions::KINDS
        )));
    }
    let spec = MatrixSpec::parse(&req.spec).map_err(AppError::BadRequest)?;
    let jdm = spec.to_jdm().map_err(AppError::BadRequest)?;
    if let Some(code) = &req.product_code {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM products WHERE code = $1)")
                .bind(code)
                .fetch_one(&state.pool)
                .await?;
        if !exists {
            return Err(AppError::BadRequest(format!("unknown productCode {code}")));
        }
    }

    let mut tx = state.pool.begin().await?;
    let row = sqlx::query_as::<_, MatrixRow>(&format!(
        "INSERT INTO matrices (key, kind, product_code, status, spec, jdm)
             VALUES ($1, $2, $3, 'draft', $4, $5)
             ON CONFLICT (key) WHERE status = 'draft'
             DO UPDATE SET kind = EXCLUDED.kind,
                           product_code = EXCLUDED.product_code,
                           spec = EXCLUDED.spec,
                           jdm = EXCLUDED.jdm,
                           updated_at = now()
             RETURNING {ROW_COLS}"
    ))
    .bind(&key)
    .bind(&kind)
    .bind(&req.product_code)
    .bind(&req.spec)
    .bind(&jdm)
    .fetch_one(&mut *tx)
    .await?;
    crate::audit::record(
        &mut tx,
        &principal,
        crate::audit::Event {
            action: crate::audit::action::ARTIFACT_PUT,
            artifact_table: Some("matrices"),
            artifact_key: Some(key.clone()),
            detail: json!({
                "kind": kind,
                "productCode": req.product_code,
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
    let n = sqlx::query("DELETE FROM matrices WHERE key = $1 AND status = 'draft'")
        .bind(&key)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound(format!("draft matrix {key}")));
    }
    crate::audit::record(
        &mut tx,
        &principal,
        crate::audit::Event {
            action: crate::audit::action::ARTIFACT_DELETE,
            artifact_table: Some("matrices"),
            artifact_key: Some(key),
            detail: json!({}),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulateRequest {
    /// Matrix spec (same shape as PUT) evaluated directly — no publish needed.
    pub spec: Value,
    /// The context to evaluate against (loan model shape or anything else).
    pub context: Value,
}

/// Editor preview: compiles the spec, evaluates it against `context`, and
/// returns the result plus per-cell match detail for debugging.
pub async fn simulate(Json(req): Json<SimulateRequest>) -> Result<Json<Value>, AppError> {
    super::validate::validate_spec_size(&req.spec, "spec")?;
    let result = matrix::simulate(&req.spec, &req.context)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::to_value(result).unwrap_or_default()))
}
