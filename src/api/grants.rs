//! Admin surface for `role_grants` — grant and revoke roles without SQL
//! (MPE-120). All three routes sit behind the admin gate in `api::router`.
//! Mutations write an attributed `grant.put`/`grant.delete` audit row inside
//! the same transaction and invalidate the local role cache post-commit, so
//! grants take effect on the caller's very next request.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::FromRow;

use crate::api::AppState;
use crate::audit;
use crate::auth::Principal;
use crate::error::AppError;

const SUBJECT_TYPES: [&str; 3] = ["sub", "email", "aud"];
const ROLES: [&str; 4] = ["viewer", "pricer", "admin", "service"];
const MAX_SUBJECT_LEN: usize = 256;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    pub limit: Option<i64>,
}

#[derive(FromRow)]
struct GrantRow {
    id: i64,
    subject_type: String,
    subject: String,
    role: String,
    granted_at: DateTime<Utc>,
    granted_by: String,
    envs: Option<Vec<String>>,
}

impl GrantRow {
    fn into_json(self) -> Value {
        json!({
            "id": self.id,
            "subjectType": self.subject_type,
            "subject": self.subject,
            "role": self.role,
            "grantedAt": self.granted_at,
            "grantedBy": self.granted_by,
            "envs": self.envs,
        })
    }
}

/// `GET /api/admin/grants` — every grant, ordered by subject type then value.
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = q.limit.unwrap_or(500).clamp(1, 1000);
    let rows = sqlx::query_as::<_, GrantRow>(
        "SELECT id, subject_type, subject, role, granted_at, granted_by, envs
         FROM role_grants ORDER BY subject_type, subject LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    let grants: Vec<Value> = rows.into_iter().map(GrantRow::into_json).collect();
    Ok(Json(json!({ "grants": grants })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertGrant {
    pub subject_type: String,
    pub subject: String,
    pub role: String,
    /// Optional environment lanes this grant permits for eval (MPE-153).
    /// Omitted = unrestricted (NULL). An empty array permits no lanes.
    pub envs: Option<Vec<String>>,
}

struct ValidatedGrant {
    subject_type: String,
    subject: String,
    role: String,
    envs: Option<Vec<String>>,
}

fn validate(req: &UpsertGrant) -> Result<ValidatedGrant, AppError> {
    let envs = req
        .envs
        .as_ref()
        .map(|envs| {
            envs.iter()
                .map(|e| {
                    crate::rules::loader::Env::parse(e)
                        .map(|env| env.as_str().to_string())
                        .ok_or_else(|| {
                            AppError::BadRequest(format!(
                                "envs entries must be one of dev|qa|uat|prod: {e:?}"
                            ))
                        })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    if !SUBJECT_TYPES.contains(&req.subject_type.as_str()) {
        return Err(AppError::BadRequest(format!(
            "subjectType must be one of {SUBJECT_TYPES:?}"
        )));
    }
    let subject = req.subject.trim();
    if subject.is_empty() || subject.len() > MAX_SUBJECT_LEN {
        return Err(AppError::BadRequest(format!(
            "subject must be 1..={MAX_SUBJECT_LEN} characters"
        )));
    }
    if req.subject_type == "email" && !subject.contains('@') {
        return Err(AppError::BadRequest(
            "subject must look like an email address when subjectType is 'email'".into(),
        ));
    }
    if !ROLES.contains(&req.role.as_str()) {
        return Err(AppError::BadRequest(format!(
            "role must be one of {ROLES:?}"
        )));
    }
    Ok(ValidatedGrant {
        subject_type: req.subject_type.clone(),
        subject: subject.to_string(),
        role: req.role.clone(),
        envs,
    })
}

/// `PUT /api/admin/grants` — insert or refresh a grant. `granted_by` is
/// stamped from the caller, never the body. 201 on insert, 200 on refresh.
pub async fn upsert(
    State(state): State<AppState>,
    principal: Principal,
    Json(req): Json<UpsertGrant>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let grant = validate(&req)?;
    let granted_by = principal
        .email
        .clone()
        .unwrap_or_else(|| principal.sub.clone());

    let mut tx = state.pool.begin().await?;
    let row = sqlx::query_as::<_, GrantRow>(
        "INSERT INTO role_grants (subject_type, subject, role, granted_by, envs)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (subject_type, subject, role)
         DO UPDATE SET granted_at = now(), granted_by = EXCLUDED.granted_by,
                       envs = EXCLUDED.envs
         RETURNING id, subject_type, subject, role, granted_at, granted_by, envs",
    )
    .bind(&grant.subject_type)
    .bind(&grant.subject)
    .bind(&grant.role)
    .bind(&granted_by)
    .bind(&grant.envs)
    .fetch_one(&mut *tx)
    .await?;
    // xmax = 0 on the RETURNING row means this was an INSERT, not an update.
    let created: bool = sqlx::query_scalar("SELECT (xmax = 0) FROM role_grants WHERE id = $1")
        .bind(row.id)
        .fetch_one(&mut *tx)
        .await?;

    audit::record(
        &mut tx,
        &principal,
        audit::Event {
            action: audit::action::GRANT_PUT,
            artifact_table: Some("role_grants"),
            artifact_key: Some(format!("{}:{}", grant.subject_type, grant.subject)),
            detail: json!({
                "subjectType": grant.subject_type,
                "subject": grant.subject,
                "role": grant.role,
                "envs": grant.envs,
                "created": created,
            }),
        },
    )
    .await?;
    tx.commit().await?;
    if let Some(auth) = &state.auth {
        auth.roles.invalidate_all().await;
    }

    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(json!({ "grant": row.into_json(), "created": created })),
    ))
}

/// `DELETE /api/admin/grants/{id}` — revoke a grant. Refuses to remove an
/// admin grant belonging to the caller (self-lockout) or the last remaining
/// admin grant.
pub async fn revoke(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query_as::<_, (String, String, String)>(
        "SELECT subject_type, subject, role FROM role_grants
         WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((subject_type, subject, role)) = row else {
        return Err(AppError::NotFound(format!("grant {id} not found")));
    };

    if role == "admin" {
        let own = principal
            .identity_keys
            .iter()
            .any(|(t, s)| *t == subject_type && *s == subject);
        if own {
            return Err(AppError::Conflict(
                "cannot revoke your own admin grant".into(),
            ));
        }
        let admins: i64 =
            sqlx::query_scalar("SELECT count(*) FROM role_grants WHERE role = 'admin'")
                .fetch_one(&mut *tx)
                .await?;
        if admins <= 1 {
            return Err(AppError::Conflict(
                "cannot revoke the last admin grant".into(),
            ));
        }
    }

    sqlx::query("DELETE FROM role_grants WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        &principal,
        audit::Event {
            action: audit::action::GRANT_DELETE,
            artifact_table: Some("role_grants"),
            artifact_key: Some(format!("{subject_type}:{subject}")),
            detail: json!({
                "id": id,
                "subjectType": subject_type,
                "subject": subject,
                "role": role,
            }),
        },
    )
    .await?;
    tx.commit().await?;
    if let Some(auth) = &state.auth {
        auth.roles.invalidate_all().await;
    }

    Ok(Json(json!({ "revoked": true, "id": id })))
}
