mod api;
mod config;
mod db;
mod domain;
mod error;
mod pricing;
mod rules;
mod seed;

use std::sync::Arc;

use anyhow::Context;
use axum::http::HeaderValue;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::rules::snapshot::SnapshotStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,mpex=debug".into()),
        )
        .json()
        .init();

    let cfg = config::Config::from_env();
    let pool = db::connect(&cfg.database_url).await.context("db connect")?;
    db::migrate(&pool).await.context("db migrate")?;
    tracing::info!("database migrated");

    if std::env::var("RUN_SEED").is_ok() {
        let dir = std::env::var("SEED_DIR").unwrap_or_else(|_| "seed".into());
        seed::run(&pool, &dir).await.context("seed")?;
    }

    let snapshots = Arc::new(SnapshotStore::new(pool.clone()));

    let cors = CorsLayer::new()
        .allow_origin(
            cfg.cors_origin
                .parse::<HeaderValue>()
                .context("invalid CORS_ORIGIN")?,
        )
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any);

    let app = api::router(api::AppState { pool, snapshots })
        .layer(TraceLayer::new_for_http())
        .layer(cors);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", cfg.port)).await?;
    tracing::info!(port = cfg.port, "mpex listening");
    axum::serve(listener, app).await?;
    Ok(())
}
