mod auth;
mod decisions;
mod evaluate;
mod expressions;
mod matrices;
mod products;
mod publications;
mod rate_sheets;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use std::sync::Arc;

use serde_json::{json, Value};
use sqlx::PgPool;

use crate::error::AppError;
use crate::rules::snapshot::SnapshotStore;

/// Admin listing revision selector: None = working drafts, Some(ts) = the
/// published snapshot as of that timestamp ("draft" also maps to None).
fn parse_admin_as_of(
    as_of: Option<&str>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    match as_of {
        None | Some("draft") => Ok(None),
        Some(ts) => chrono::DateTime::parse_from_rfc3339(ts)
            .map(|t| Some(t.with_timezone(&chrono::Utc)))
            .map_err(|_| {
                AppError::BadRequest(format!("asOf must be 'draft' or RFC3339, got {ts:?}"))
            }),
    }
}

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub snapshots: Arc<SnapshotStore>,
    /// OIDC context — `None` keeps the API runnable without an IdP;
    /// auth extractors then reject with an error (enforcement lands in
    /// MPE-99 alongside per-route role gates).
    pub auth: Option<Arc<crate::auth::AuthContext>>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/health", get(health))
        .route("/api/products", get(products::list))
        .route("/api/auth/whoami", get(auth::whoami))
        .route("/api/evaluate/eligibility", post(evaluate::eligibility))
        .route("/api/evaluate/pricing", post(evaluate::pricing))
        .route(
            "/api/validate/expression",
            post(evaluate::validate_expression),
        )
        .route("/api/decisions", get(decisions::list))
        .route(
            "/api/decisions/{key}",
            get(decisions::get)
                .put(decisions::put)
                .delete(decisions::delete),
        )
        .route("/api/expressions", get(expressions::list))
        .route(
            "/api/expressions/{key}",
            get(expressions::get)
                .put(expressions::put)
                .delete(expressions::delete),
        )
        .route("/api/matrices", get(matrices::list))
        .route(
            "/api/matrices/{key}",
            get(matrices::get)
                .put(matrices::put)
                .delete(matrices::delete),
        )
        .route("/api/simulate/matrix", post(matrices::simulate))
        .route("/api/rate-sheets", get(rate_sheets::list))
        .route(
            "/api/rate-sheets/{key}",
            get(rate_sheets::get)
                .put(rate_sheets::put)
                .delete(rate_sheets::delete),
        )
        .route(
            "/api/publications",
            get(publications::list).post(publications::create),
        )
        .route("/api/publications/{id}", get(publications::get))
        .with_state(state)
}

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn health(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(json!({ "status": "ok", "db": "up" })))
}
