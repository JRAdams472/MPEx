use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::api::AppState;
use crate::error::AppError;
use crate::rules::overrides;

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
    let overrides = match super::parse_admin_as_of(q.as_of.as_deref())? {
        None => {
            let rows =
                sqlx::query_as::<_, (String, String, String, String, String, DateTime<Utc>, bool)>(
                    "SELECT o.key, o.target_kind, o.target_key, o.value_path, o.description,
                        o.updated_at,
                        EXISTS (SELECT 1 FROM overrides p
                                WHERE p.key = o.key AND p.status = 'published'
                                  AND p.env = 'dev')
                 FROM overrides o WHERE o.status = 'draft' ORDER BY o.key",
                )
                .fetch_all(&state.pool)
                .await?;
            rows.into_iter()
                .map(
                    |(
                        key,
                        target_kind,
                        target_key,
                        value_path,
                        description,
                        updated_at,
                        has_published,
                    )| {
                        json!({
                            "key": key,
                            "targetKind": target_kind,
                            "targetKey": target_key,
                            "valuePath": value_path,
                            "description": description,
                            "status": "draft",
                            "updatedAt": updated_at,
                            "hasPublished": has_published,
                        })
                    },
                )
                .collect::<Vec<_>>()
        }
        Some(ts) => {
            let rows = sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    String,
                    String,
                    Option<DateTime<Utc>>,
                    Option<Uuid>,
                ),
            >(
                "SELECT DISTINCT ON (key) key, target_kind, target_key, value_path,
                        description, published_at, publication_id
                 FROM overrides
                 WHERE status = 'published' AND published_at <= $1 AND env = $2
                 ORDER BY key, published_at DESC",
            )
            .bind(ts)
            .bind(env.as_str())
            .fetch_all(&state.pool)
            .await?;
            rows.into_iter()
                .map(
                    |(
                        key,
                        target_kind,
                        target_key,
                        value_path,
                        description,
                        published_at,
                        publication_id,
                    )| {
                        json!({
                            "key": key,
                            "targetKind": target_kind,
                            "targetKey": target_key,
                            "valuePath": value_path,
                            "description": description,
                            "status": "published",
                            "publishedAt": published_at,
                            "publicationId": publication_id,
                        })
                    },
                )
                .collect()
        }
    };
    Ok(Json(json!({ "overrides": overrides })))
}

type OverrideRow = (
    Uuid,
    String,
    String,
    String,
    String,
    Value,
    String,
    String,
    Option<DateTime<Utc>>,
    Option<Uuid>,
    String,
    DateTime<Utc>,
    DateTime<Utc>,
);

const ROW_COLS: &str = "id, key, target_kind, target_key, value_path, value, \
     description, status, published_at, publication_id, env, created_at, updated_at";

fn row_json(
    (
        id,
        key,
        target_kind,
        target_key,
        value_path,
        value,
        description,
        status,
        published_at,
        publication_id,
        env,
        created_at,
        updated_at,
    ): OverrideRow,
) -> Value {
    json!({
        "id": id,
        "key": key,
        "targetKind": target_kind,
        "targetKey": target_key,
        "valuePath": value_path,
        "value": value,
        "description": description,
        "status": status,
        "publishedAt": published_at,
        "publicationId": publication_id,
        "env": env,
        "createdAt": created_at,
        "updatedAt": updated_at,
    })
}

pub async fn get(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<Value>, AppError> {
    super::validate::validate_key(&key)?;
    let draft = sqlx::query_as::<_, OverrideRow>(&format!(
        "SELECT {ROW_COLS} FROM overrides WHERE key = $1 AND status = 'draft'"
    ))
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?;
    let published = sqlx::query_as::<_, OverrideRow>(&format!(
        "SELECT {ROW_COLS} FROM overrides
             WHERE key = $1 AND status = 'published' AND env = 'dev'
             ORDER BY published_at DESC LIMIT 1"
    ))
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?;
    if draft.is_none() && published.is_none() {
        return Err(AppError::NotFound(format!("override {key}")));
    }

    let history =
        sqlx::query_as::<_, (Option<DateTime<Utc>>, Option<Uuid>, Option<String>, String)>(
            "SELECT o.published_at, o.publication_id, p.label, o.env
         FROM overrides o LEFT JOIN publications p ON p.id = o.publication_id
         WHERE o.key = $1 AND o.status = 'published'
         ORDER BY o.published_at DESC",
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

/// The document a target resolves to for patching, as the working set sees
/// it (draft first, else latest published in dev). Expressions patch a
/// synthetic `{"expression": <text>}` doc — the second tuple element carries
/// the row's kind so a patched expression re-compiles under the right mode.
async fn target_doc(
    pool: &PgPool,
    kind: &str,
    key: &str,
) -> Result<Option<(Value, Option<String>)>, AppError> {
    match kind {
        "decision" => {
            let row = sqlx::query_scalar::<_, Value>(
                "SELECT jdm FROM (
                   SELECT jdm, 0 AS prio FROM decisions
                   WHERE key = $1 AND status = 'draft'
                   UNION ALL
                   SELECT jdm, 1 FROM (
                     SELECT DISTINCT ON (key) jdm FROM decisions
                     WHERE key = $1 AND status = 'published' AND env = 'dev'
                     ORDER BY key, published_at DESC
                   ) p
                 ) t ORDER BY prio LIMIT 1",
            )
            .bind(key)
            .fetch_optional(pool)
            .await?;
            Ok(row.map(|jdm| (jdm, None)))
        }
        "matrix" => {
            let row = sqlx::query_scalar::<_, Value>(
                "SELECT spec FROM (
                   SELECT spec, 0 AS prio FROM matrices
                   WHERE key = $1 AND status = 'draft'
                   UNION ALL
                   SELECT spec, 1 FROM (
                     SELECT DISTINCT ON (key) spec FROM matrices
                     WHERE key = $1 AND status = 'published' AND env = 'dev'
                     ORDER BY key, published_at DESC
                   ) p
                 ) t ORDER BY prio LIMIT 1",
            )
            .bind(key)
            .fetch_optional(pool)
            .await?;
            Ok(row.map(|spec| (spec, None)))
        }
        "rate_sheet" => {
            let row = sqlx::query_scalar::<_, Value>(
                "SELECT spec FROM (
                   SELECT spec, 0 AS prio FROM rate_sheets
                   WHERE key = $1 AND status = 'draft'
                   UNION ALL
                   SELECT spec, 1 FROM (
                     SELECT DISTINCT ON (key) spec FROM rate_sheets
                     WHERE key = $1 AND status = 'published' AND env = 'dev'
                     ORDER BY key, published_at DESC
                   ) p
                 ) t ORDER BY prio LIMIT 1",
            )
            .bind(key)
            .fetch_optional(pool)
            .await?;
            Ok(row.map(|spec| (spec, None)))
        }
        "expression" => {
            let row = sqlx::query_as::<_, (String, String)>(
                "SELECT expression, kind FROM (
                   SELECT expression, kind, 0 AS prio FROM expressions
                   WHERE key = $1 AND status = 'draft'
                   UNION ALL
                   SELECT expression, kind, 1 FROM (
                     SELECT DISTINCT ON (key) expression, kind FROM expressions
                     WHERE key = $1 AND status = 'published' AND env = 'dev'
                     ORDER BY key, published_at DESC
                   ) p
                 ) t ORDER BY prio LIMIT 1",
            )
            .bind(key)
            .fetch_optional(pool)
            .await?;
            Ok(row.map(|(expr, kind)| (overrides::expression_doc(&expr), Some(kind))))
        }
        _ => Ok(None),
    }
}

/// Prove the patched document is still a valid artifact of its kind — an
/// override that would corrupt its target is rejected at write time, not at
/// the next evaluation.
fn validate_patched(kind: &str, doc: &Value, expr_kind: Option<&str>) -> Result<(), String> {
    match kind {
        "decision" => serde_json::from_value::<zen_engine::model::DecisionContent>(doc.clone())
            .map(|_| ())
            .map_err(|e| format!("patched decision is invalid JDM: {e}")),
        "matrix" => crate::rules::matrix::MatrixSpec::parse(doc)
            .and_then(|m| m.to_jdm().map(|_| ()))
            .map_err(|e| format!("patched matrix spec invalid: {e}")),
        "rate_sheet" => crate::rules::rate_sheet::RateSheetSpec::parse(doc)
            .map(|_| ())
            .map_err(|e| format!("patched rate-sheet spec invalid: {e}")),
        "expression" => {
            let source = overrides::expression_from_doc(doc)?;
            super::validate::validate_expression_source(source)
                .map_err(|e| format!("patched expression: {e}"))?;
            match expr_kind.unwrap_or("standard") {
                "unary" => zen_expression::compile_unary_expression(source)
                    .map(|_| ())
                    .map_err(|e| format!("patched expression does not compile (unary): {e}")),
                _ => zen_expression::compile_expression(source)
                    .map(|_| ())
                    .map_err(|e| format!("patched expression does not compile: {e}")),
            }
        }
        _ => Ok(()),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertOverride {
    pub target_kind: String,
    pub target_key: String,
    /// RFC 6901 JSON Pointer into the target's patchable document.
    pub value_path: String,
    /// Scalar leaf replacement — strings may carry ZEN expression text.
    pub value: Value,
    pub description: Option<String>,
}

/// Upsert the mutable draft for `key`. The target must resolve in the
/// current working set (draft else latest dev-published) and the patched
/// document must still be a valid artifact — both checks run against real
/// content here, so a bad surgical change fails fast instead of poisoning
/// the next publish.
pub async fn put(
    State(state): State<AppState>,
    principal: crate::auth::Principal,
    Path(key): Path<String>,
    Json(req): Json<UpsertOverride>,
) -> Result<Json<Value>, AppError> {
    super::validate::validate_key(&key)?;
    super::validate::validate_key(&req.target_key)?;
    if !overrides::TARGET_KINDS.contains(&req.target_kind.as_str()) {
        return Err(AppError::BadRequest(format!(
            "targetKind must be one of {:?}, got {:?}",
            overrides::TARGET_KINDS,
            req.target_kind
        )));
    }
    overrides::validate_path(&req.value_path).map_err(AppError::BadRequest)?;
    if !overrides::is_scalar(&req.value) {
        return Err(AppError::BadRequest(
            "value must be a scalar leaf (string, number, bool, null)".into(),
        ));
    }

    let (mut doc, expr_kind) = target_doc(&state.pool, &req.target_kind, &req.target_key)
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "target {}:{} does not resolve in the current working set",
                req.target_kind, req.target_key
            ))
        })?;
    overrides::apply(&mut doc, &req.value_path, &req.value).map_err(AppError::BadRequest)?;
    validate_patched(&req.target_kind, &doc, expr_kind.as_deref()).map_err(AppError::BadRequest)?;

    let mut tx = state.pool.begin().await?;
    let row = sqlx::query_as::<_, OverrideRow>(&format!(
        "INSERT INTO overrides
             (key, target_kind, target_key, value_path, value, description, status)
             VALUES ($1, $2, $3, $4, $5, $6, 'draft')
             ON CONFLICT (key) WHERE status = 'draft'
             DO UPDATE SET target_kind = EXCLUDED.target_kind,
                           target_key = EXCLUDED.target_key,
                           value_path = EXCLUDED.value_path,
                           value = EXCLUDED.value,
                           description = EXCLUDED.description,
                           updated_at = now()
             RETURNING {ROW_COLS}"
    ))
    .bind(&key)
    .bind(&req.target_kind)
    .bind(&req.target_key)
    .bind(&req.value_path)
    .bind(&req.value)
    .bind(req.description.unwrap_or_default())
    .fetch_one(&mut *tx)
    .await?;
    crate::audit::record(
        &mut tx,
        &principal,
        crate::audit::Event {
            action: crate::audit::action::OVERRIDE_PUT,
            artifact_table: Some("overrides"),
            artifact_key: Some(key.clone()),
            detail: json!({
                "targetKind": req.target_kind,
                "targetKey": req.target_key,
                "valuePath": req.value_path,
                "contentHash": crate::audit::content_hash(&req.value),
            }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(row_json(row)))
}

/// Deletes the draft only; a published override stays in history and keeps
/// applying to published/asOf snapshots — same immutability rule as every
/// other artifact kind.
pub async fn delete(
    State(state): State<AppState>,
    principal: crate::auth::Principal,
    Path(key): Path<String>,
) -> Result<StatusCode, AppError> {
    super::validate::validate_key(&key)?;
    let mut tx = state.pool.begin().await?;
    let n = sqlx::query("DELETE FROM overrides WHERE key = $1 AND status = 'draft'")
        .bind(&key)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound(format!("draft override {key}")));
    }
    crate::audit::record(
        &mut tx,
        &principal,
        crate::audit::Event {
            action: crate::audit::action::OVERRIDE_DELETE,
            artifact_table: Some("overrides"),
            artifact_key: Some(key),
            detail: json!({}),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
