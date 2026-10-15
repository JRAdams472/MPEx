use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::error::AppError;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/health", get(health))
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
