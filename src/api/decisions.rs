use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;
use zen_engine::model::DecisionContent;

use crate::api::AppState;
use crate::error::AppError;

pub(crate) const KINDS: [&str; 4] = ["eligibility", "llpa", "pricing", "shared"];

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
    let decisions = match super::parse_admin_as_of(q.as_of.as_deref())? {
        None => {
            let rows = sqlx::query_as::<_, (String, String, Option<String>, DateTime<Utc>, bool)>(
                "SELECT d.key, d.kind, d.product_code, d.updated_at,
                        EXISTS (SELECT 1 FROM decisions p
                                WHERE p.key = d.key AND p.status = 'published'
                                  AND p.env = 'dev')
                 FROM decisions d WHERE d.status = 'draft' ORDER BY d.key",
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
                 FROM decisions
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
    Ok(Json(json!({ "decisions": decisions })))
}

type DecisionRow = (
    Uuid,
    String,
    String,
    Option<String>,
    String,
    Option<DateTime<Utc>>,
    Option<Uuid>,
    Value,
    DateTime<Utc>,
    DateTime<Utc>,
);

const ROW_COLS: &str =
    "id, key, kind, product_code, status, published_at, publication_id, jdm, created_at, updated_at";

fn row_json(
    (
        id,
        key,
        kind,
        product_code,
        status,
        published_at,
        publication_id,
        jdm,
        created_at,
        updated_at,
    ): DecisionRow,
) -> Value {
    json!({
        "id": id,
        "key": key,
        "kind": kind,
        "productCode": product_code,
        "status": status,
        "publishedAt": published_at,
        "publicationId": publication_id,
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
    let draft = sqlx::query_as::<_, DecisionRow>(&format!(
        "SELECT {ROW_COLS} FROM decisions WHERE key = $1 AND status = 'draft'"
    ))
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?;
    let published = sqlx::query_as::<_, DecisionRow>(&format!(
        "SELECT {ROW_COLS} FROM decisions
             WHERE key = $1 AND status = 'published' AND env = 'dev'
             ORDER BY published_at DESC LIMIT 1"
    ))
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?;
    if draft.is_none() && published.is_none() {
        return Err(AppError::NotFound(format!("decision {key}")));
    }

    let history =
        sqlx::query_as::<_, (Option<DateTime<Utc>>, Option<Uuid>, Option<String>, String)>(
            "SELECT d.published_at, d.publication_id, p.label, d.env
         FROM decisions d LEFT JOIN publications p ON p.id = d.publication_id
         WHERE d.key = $1 AND d.status = 'published'
         ORDER BY d.published_at DESC",
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
pub struct UpsertDecision {
    /// Artifact kind; defaults to "shared" (PUT fully replaces the draft).
    pub kind: Option<String>,
    pub product_code: Option<String>,
    pub jdm: Value,
}

/// Upsert the mutable draft for `key`. Published rows are never touched —
/// they change only through publish.
pub async fn put(
    State(state): State<AppState>,
    principal: crate::auth::Principal,
    Path(key): Path<String>,
    Json(req): Json<UpsertDecision>,
) -> Result<Json<Value>, AppError> {
    super::validate::validate_key(&key)?;
    super::validate::validate_spec_size(&req.jdm, "jdm")?;
    if let Some(code) = &req.product_code {
        super::validate::validate_key(code)?;
    }
    let kind = req.kind.unwrap_or_else(|| "shared".into());
    if !KINDS.contains(&kind.as_str()) {
        return Err(AppError::BadRequest(format!(
            "kind must be one of {KINDS:?}, got {kind:?}"
        )));
    }
    if req.jdm.is_null() || !req.jdm.is_object() {
        return Err(AppError::BadRequest(
            "jdm must be a decision graph object".into(),
        ));
    }
    serde_json::from_value::<DecisionContent>(req.jdm.clone())
        .map_err(|e| AppError::BadRequest(format!("invalid JDM: {e}")))?;
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
    let row = sqlx::query_as::<_, DecisionRow>(&format!(
        "INSERT INTO decisions (key, kind, product_code, status, jdm)
             VALUES ($1, $2, $3, 'draft', $4)
             ON CONFLICT (key) WHERE status = 'draft'
             DO UPDATE SET kind = EXCLUDED.kind,
                           product_code = EXCLUDED.product_code,
                           jdm = EXCLUDED.jdm,
                           updated_at = now()
             RETURNING {ROW_COLS}"
    ))
    .bind(&key)
    .bind(&kind)
    .bind(&req.product_code)
    .bind(&req.jdm)
    .fetch_one(&mut *tx)
    .await?;
    crate::audit::record(
        &mut tx,
        &principal,
        crate::audit::Event {
            action: crate::audit::action::ARTIFACT_PUT,
            artifact_table: Some("decisions"),
            artifact_key: Some(key.clone()),
            detail: json!({
                "kind": kind,
                "productCode": req.product_code,
                "contentHash": crate::audit::content_hash(&req.jdm),
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
    let n = sqlx::query("DELETE FROM decisions WHERE key = $1 AND status = 'draft'")
        .bind(&key)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound(format!("draft decision {key}")));
    }
    crate::audit::record(
        &mut tx,
        &principal,
        crate::audit::Event {
            action: crate::audit::action::ARTIFACT_DELETE,
            artifact_table: Some("decisions"),
            artifact_key: Some(key),
            detail: json!({}),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
