use std::sync::Arc;

use anyhow::Context;
use axum::http::{header, HeaderValue, Method};
use mpex::rules::snapshot::SnapshotStore;
use mpex::{api, config, db, seed, telemetry};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let cfg = config::Config::from_env();
    let telemetry_guard = telemetry::init(&cfg).context("telemetry init")?;

    let pool = db::connect(&cfg.database_url).await.context("db connect")?;
    db::migrate(&pool).await.context("db migrate")?;
    tracing::info!("database migrated");

    if std::env::var("RUN_SEED").is_ok() {
        let dir = std::env::var("SEED_DIR").unwrap_or_else(|_| "seed".into());
        seed::run(&pool, &dir).await.context("seed")?;
    }

    let snapshots = Arc::new(SnapshotStore::new(pool.clone()));
    let auth = mpex::auth::init(&cfg, pool.clone())
        .await
        .context("auth init")?;

    let cors = CorsLayer::new()
        .allow_origin(
            cfg.cors_origin
                .parse::<HeaderValue>()
                .context("invalid CORS_ORIGIN")?,
        )
        // Explicit surface, not Any: the BFF proxies browser traffic, so
        // cross-origin callers only need these verbs/headers.
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    // Per-IP limits come from env (None = RATE_LIMIT_DISABLE); they ride in
    // AppState so test routers simply leave them unset and un-throttled.
    let rate_limits = mpex::limits::EdgeLimits::from_env();

    let app = api::router(api::AppState {
        pool,
        snapshots,
        auth,
        rate_limits,
    })
    // INFO so request spans survive the EnvFilter and reach the OTLP
    // trace exporter (the DefaultMakeSpan default is DEBUG).
    .layer(
        TraceLayer::new_for_http()
            .make_span_with(tower_http::trace::DefaultMakeSpan::new().level(tracing::Level::INFO)),
    )
    .layer(cors);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", cfg.port)).await?;
    tracing::info!(port = cfg.port, "mpex listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    telemetry_guard.shutdown();
    Ok(())
}
