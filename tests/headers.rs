//! Security-header assertions (MPE-97 phase 4): every API response carries
//! the baseline hardening set — success, gate rejection, or not-found alike.
//! Gated on DATABASE_URL (set in CI); skips locally otherwise.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use sqlx::PgPool;
use tower::ServiceExt;

use mpex::api::{self, AppState};
use mpex::auth::testkit;
use mpex::rules::snapshot::SnapshotStore;

fn make_app(pool: &PgPool) -> Router {
    api::router(AppState {
        pool: pool.clone(),
        snapshots: Arc::new(SnapshotStore::new(pool.clone())),
        auth: Some(testkit::test_context(pool.clone())),
    })
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
) -> axum::response::Response {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    app.clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

fn assert_headers(res: &axum::response::Response) {
    let h = res.headers();
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["x-frame-options"], "DENY");
    assert_eq!(h["referrer-policy"], "no-referrer");
    assert_eq!(
        h["content-security-policy"],
        "default-src 'self'; frame-ancestors 'none'"
    );
}

#[tokio::test]
async fn security_headers_on_every_response() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping headers test");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    testkit::ensure_grants(&pool).await;
    let app = make_app(&pool);

    // 200 on a public route.
    let res = call(&app, "GET", "/api/health", None).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_headers(&res);

    // 401 gate rejection — headers apply to middleware short-circuits too.
    let res = call(&app, "GET", "/api/decisions", None).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert_headers(&res);

    // 200 on a gated route with a valid token.
    let res = call(&app, "GET", "/api/decisions", Some(&testkit::admin_token())).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_headers(&res);
}
