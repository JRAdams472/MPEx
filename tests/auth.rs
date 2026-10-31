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
        rate_limits: None,
        rate_index: None,
    })
}

async fn whoami(app: &Router, token: Option<&str>) -> (StatusCode, Value) {
    call(app, "GET", "/api/auth/whoami", token, None).await
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let res = app
        .clone()
        .oneshot(
            req.body(match body {
                Some(b) => Body::from(serde_json::to_vec(&b).unwrap()),
                None => Body::empty(),
            })
            .unwrap(),
        )
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
    testkit::ensure_grants(&pool).await;
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
        "INSERT INTO role_grants (subject_type, subject, role, granted_by) VALUES
         ('sub', 'auth-test-sub', 'pricer', 'auth-test'),
         ('email', 'authtest@mpex.dev', 'viewer', 'auth-test')
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
        "INSERT INTO role_grants (subject_type, subject, role, granted_by)
         VALUES ('aud', $1, 'service', 'auth-test') ON CONFLICT DO NOTHING",
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

/// The Phase-2 enforcement contract: every /api/* route (except health)
/// rejects unauthenticated callers with 401; an authenticated caller with
/// insufficient role gets 403. `Service` is standalone — evaluate + products
/// only, even viewer routes are off-limits.
#[tokio::test]
async fn role_gates_enforce_401_and_403() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping auth test");
        return;
    };
    let app = make_app(&pool);
    let (viewer, pricer, admin, auditor, svc) = (
        testkit::viewer_token(),
        testkit::pricer_token(),
        testkit::admin_token(),
        testkit::auditor_token(),
        testkit::svc_token(),
    );
    // A validly-signed token whose subject has zero grants.
    let nobody = testkit::mint("nobody", 3600, &[]);

    // Health stays public — no token at all.
    let (status, _) = call(&app, "GET", "/healthz", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&app, "GET", "/api/health", None, None).await;
    assert_eq!(status, StatusCode::OK);

    // (method, uri, allowed, denied) — allowed/denied are token labels.
    // "Allowed" means the gate passes: any non-401/403 status (the handler
    // may still 4xx the empty body — that's downstream of auth).
    let cases: &[(&str, &str, &[&str], &[&str])] = &[
        ("GET", "/api/products", &["v", "p", "a", "u", "s"], &[]),
        (
            "POST",
            "/api/evaluate/eligibility",
            &["v", "p", "a", "u", "s"],
            &[],
        ),
        (
            "POST",
            "/api/evaluate/pricing",
            &["v", "p", "a", "u", "s"],
            &[],
        ),
        (
            "POST",
            "/api/validate/expression",
            &["v", "p", "a", "u"],
            &["s"],
        ),
        (
            "POST",
            "/api/simulate/matrix",
            &["v", "p", "a", "u"],
            &["s"],
        ),
        ("GET", "/api/decisions", &["v", "p", "a", "u"], &["s"]),
        ("GET", "/api/decisions/x", &["v", "p", "a", "u"], &["s"]),
        ("GET", "/api/expressions", &["v", "p", "a", "u"], &["s"]),
        ("GET", "/api/matrices", &["v", "p", "a", "u"], &["s"]),
        ("GET", "/api/rate-sheets", &["v", "p", "a", "u"], &["s"]),
        ("GET", "/api/publications", &["v", "p", "a", "u"], &["s"]),
        ("PUT", "/api/decisions/x", &["p", "a"], &["v", "u", "s"]),
        ("DELETE", "/api/decisions/x", &["p", "a"], &["v", "u", "s"]),
        ("PUT", "/api/expressions/x", &["p", "a"], &["v", "u", "s"]),
        ("DELETE", "/api/matrices/x", &["p", "a"], &["v", "u", "s"]),
        ("PUT", "/api/rate-sheets/x", &["p", "a"], &["v", "u", "s"]),
        (
            "PATCH",
            "/api/matrices/x/cells",
            &["p", "a"],
            &["v", "u", "s"],
        ),
        (
            "PATCH",
            "/api/rate-sheets/x/cells",
            &["p", "a"],
            &["v", "u", "s"],
        ),
        ("POST", "/api/publications", &["a"], &["v", "p", "u", "s"]),
        ("GET", "/api/audit", &["a", "u"], &["v", "p", "s"]),
        ("GET", "/api/audit/verify", &["a", "u"], &["v", "p", "s"]),
        ("GET", "/api/admin/grants", &["a"], &["v", "p", "u", "s"]),
        ("PUT", "/api/admin/grants", &["a"], &["v", "p", "u", "s"]),
        (
            "DELETE",
            "/api/admin/grants/1",
            &["a"],
            &["v", "p", "u", "s"],
        ),
    ];
    let token_for = |l: &str| match l {
        "v" => &viewer,
        "p" => &pricer,
        "a" => &admin,
        "u" => &auditor,
        "s" => &svc,
        _ => unreachable!(),
    };

    for (method, uri, allowed, denied) in cases {
        // Unauthenticated → 401 (except health, covered above).
        let (status, body) = call(&app, method, uri, None, None).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} unauthenticated should 401: {body}"
        );
        // Zero-role principal → 403 everywhere gated.
        let (status, body) = call(&app, method, uri, Some(&nobody), None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {uri} zero-role should 403: {body}"
        );
        for l in *denied {
            let (status, body) = call(&app, method, uri, Some(token_for(l)), None).await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "{method} {uri} role {l} should 403: {body}"
            );
        }
        for l in *allowed {
            let (status, body) = call(&app, method, uri, Some(token_for(l)), None).await;
            assert!(
                status != StatusCode::UNAUTHORIZED && status != StatusCode::FORBIDDEN,
                "{method} {uri} role {l} should pass the gate, got {status}: {body}"
            );
        }
    }
}
