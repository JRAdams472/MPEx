//! Admin read surface for the audit trail: filtered, paginated listing plus
//! on-demand chain verification. Both routes sit behind the admin gate in
//! `api::router`.

use axum::extract::{Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::AppState;
use crate::audit;
use crate::error::AppError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub actor: Option<String>,
    pub action: Option<String>,
    pub table: Option<String>,
    pub key: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

/// `GET /api/audit` — newest-first listing with optional filters on actor,
/// action, artifact table/key, and an occurred_at range.
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);

    let mut qb = sqlx::QueryBuilder::new(
        "SELECT id, occurred_at, actor_sub, action, artifact_table,
                artifact_key, detail, prev_hash, hash
         FROM audit_events WHERE TRUE",
    );
    if let Some(actor) = &q.actor {
        qb.push(" AND actor_sub = ").push_bind(actor);
    }
    if let Some(action) = &q.action {
        qb.push(" AND action = ").push_bind(action);
    }
    if let Some(table) = &q.table {
        qb.push(" AND artifact_table = ").push_bind(table);
    }
    if let Some(key) = &q.key {
        qb.push(" AND artifact_key = ").push_bind(key);
    }
    if let Some(from) = &q.from {
        qb.push(" AND occurred_at >= ").push_bind(from);
    }
    if let Some(to) = &q.to {
        qb.push(" AND occurred_at <= ").push_bind(to);
    }
    qb.push(" ORDER BY id DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);

    let rows = qb
        .build_query_as::<AuditRow>()
        .fetch_all(&state.pool)
        .await?;
    let events: Vec<Value> = rows.into_iter().map(AuditRow::into_json).collect();
    Ok(Json(json!({ "events": events })))
}

#[derive(sqlx::FromRow)]
struct AuditRow {
    id: i64,
    occurred_at: DateTime<Utc>,
    actor_sub: String,
    action: String,
    artifact_table: Option<String>,
    artifact_key: Option<String>,
    detail: Value,
    prev_hash: String,
    hash: String,
}

impl AuditRow {
    fn into_json(self) -> Value {
        json!({
            "id": self.id,
            "occurredAt": self.occurred_at,
            "actorSub": self.actor_sub,
            "action": self.action,
            "artifactTable": self.artifact_table,
            "artifactKey": self.artifact_key,
            "detail": self.detail,
            "prevHash": self.prev_hash,
            "hash": self.hash,
        })
    }
}

/// `GET /api/audit/verify` — recompute the whole chain and report the first
/// break, if any.
pub async fn verify(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let result = audit::verify_chain(&state.pool).await?;
    Ok(Json(serde_json::to_value(result).unwrap_or_default()))
}
