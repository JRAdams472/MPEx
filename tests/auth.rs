//! Auth machinery integration test: `GET /api/auth/whoami` exercises the
//! full extractor — bearer parse, JWT validation against the in-process
//! test JWKS, and DB-backed role resolution from `role_grants`.
//! Gated on DATABASE_URL (set in CI); skips locally otherwise.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
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

async fn whoami(app: &Router, token: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder().method("GET").uri("/api/auth/whoami");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    Some(pool)
}

#[tokio::test]
async fn whoami_rejects_missing_and_bad_tokens() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping auth test");
        return;
    };
    let app = make_app(&pool);

    let (status, body) = whoami(&app, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let (status, body) = whoami(&app, Some("not-a-jwt")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // Validly signed but expired (well past the 60s leeway).
    let expired = testkit::mint("auth-test-sub", -3600, &[]);
    let (status, body) = whoami(&app, Some(&expired)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn whoami_returns_principal_with_resolved_roles() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping auth test");
        return;
    };
    let app = make_app(&pool);

    // Grant one role on the opaque `sub`, one on the email claim — the
    // resolver unions both (dex static users key on email).
    sqlx::query(
        "INSERT INTO role_grants (subject, role, granted_by) VALUES
         ('auth-test-sub', 'pricer', 'auth-test'),
         ('authtest@mpex.dev', 'viewer', 'auth-test')
         ON CONFLICT DO NOTHING",
    )
    .execute(&pool)
    .await
    .unwrap();

    let token = testkit::mint(
        "auth-test-sub",
        3600,
        &[("email".into(), json!("authtest@mpex.dev"))],
    );
    let (status, body) = whoami(&app, Some(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sub"], json!("auth-test-sub"));
    assert_eq!(body["email"], json!("authtest@mpex.dev"));
    let roles: Vec<&str> = body["roles"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r.as_str())
        .collect();
    assert!(
        roles.contains(&"pricer") && roles.contains(&"viewer"),
        "expected unioned roles: {body}"
    );

    // A subject with no grants still authenticates — with an empty role set.
    let token = testkit::mint("nobody", 3600, &[]);
    let (status, body) = whoami(&app, Some(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["roles"], json!([]));

    // Service-token path: client-credentials tokens carry the client_id in
    // `aud` (dex behavior) — the grant keys on it, not the opaque sub.
    sqlx::query(
        "INSERT INTO role_grants (subject, role, granted_by)
         VALUES ($1, 'service', 'auth-test') ON CONFLICT DO NOTHING",
    )
    .bind(testkit::SVC_AUDIENCE)
    .execute(&pool)
    .await
    .unwrap();
    let token = testkit::mint(
        "encoded-svc-sub",
        3600,
        &[("aud".into(), json!(testkit::SVC_AUDIENCE))],
    );
    let (status, body) = whoami(&app, Some(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["roles"], json!(["service"]));

    sqlx::query("DELETE FROM role_grants WHERE granted_by = 'auth-test'")
        .execute(&pool)
        .await
        .unwrap();
}
