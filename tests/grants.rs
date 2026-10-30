//! role_grants admin API integration tests (MPE-120): gating, upsert,
//! revoke, lockout guards, audit rows, and the grant → effective-role
//! change on the very next request. Gated on DATABASE_URL (set in CI);
//! skips locally otherwise.

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
    })
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

fn grant_id(list: &Value, subject_type: &str, subject: &str, role: &str) -> Option<i64> {
    list["grants"].as_array()?.iter().find_map(|g| {
        (g["subjectType"] == subject_type && g["subject"] == subject && g["role"] == role)
            .then(|| g["id"].as_i64().unwrap())
    })
}

#[tokio::test]
async fn grants_list_requires_admin() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping grants test");
        return;
    };
    let app = make_app(&pool);

    // No token → 401; viewer token → 403; service → 403.
    let (status, _) = call(&app, "GET", "/api/admin/grants", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let viewer = testkit::token(testkit::SUB_VIEWER);
    let (status, _) = call(&app, "GET", "/api/admin/grants", Some(&viewer), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let svc = testkit::token(testkit::SUB_SVC);
    let (status, _) = call(&app, "GET", "/api/admin/grants", Some(&svc), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let admin = testkit::token(testkit::SUB_ADMIN);
    let (status, body) = call(&app, "GET", "/api/admin/grants", Some(&admin), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(grant_id(&body, "sub", testkit::SUB_ADMIN, "admin").is_some());
}

#[tokio::test]
async fn grant_put_then_role_visible_next_request() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping grants test");
        return;
    };
    let app = make_app(&pool);
    let admin = testkit::token(testkit::SUB_ADMIN);

    // A principal whose only identity is a fresh email — viewer-less today.
    let email = "grants-email@mpex.dev";
    let user = testkit::mint("grants-email-sub", 3600, &[("email".into(), email.into())]);
    let (status, body) = call(&app, "GET", "/api/auth/whoami", Some(&user), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["roles"], json!([]), "{body}");

    // Grant viewer on the email claim — effective on the very next request.
    let (status, body) = call(
        &app,
        "PUT",
        "/api/admin/grants",
        Some(&admin),
        Some(json!({"subjectType": "email", "subject": email, "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["created"], true);
    let id = body["grant"]["id"].as_i64().expect("grant id");

    let (status, body) = call(&app, "GET", "/api/auth/whoami", Some(&user), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["roles"], json!(["viewer"]), "{body}");

    // Upsert refreshes in place — 200, created:false, still one row.
    let (status, body) = call(
        &app,
        "PUT",
        "/api/admin/grants",
        Some(&admin),
        Some(json!({"subjectType": "email", "subject": email, "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["created"], false);
    assert_eq!(body["grant"]["id"].as_i64().unwrap(), id);

    // Revoke → role gone on the next request; grant.delete audited.
    let (status, body) = call(
        &app,
        "DELETE",
        &format!("/api/admin/grants/{id}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revoked"], true);

    let (status, body) = call(&app, "GET", "/api/auth/whoami", Some(&user), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["roles"], json!([]), "{body}");

    // Both mutations wrote attributed audit rows.
    let (status, body) = call(
        &app,
        "GET",
        "/api/audit?action=grant.put&limit=5",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let hit = body["events"].as_array().unwrap().iter().any(|e| {
        e["artifactTable"] == "role_grants"
            && e["artifactKey"] == format!("email:{email}")
            && e["actorSub"] == testkit::SUB_ADMIN
    });
    assert!(hit, "expected a grant.put audit row for {email}");
    let (status, body) = call(
        &app,
        "GET",
        "/api/audit?action=grant.delete&limit=5",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let hit = body["events"].as_array().unwrap().iter().any(|e| {
        e["artifactTable"] == "role_grants" && e["artifactKey"] == format!("email:{email}")
    });
    assert!(hit, "expected a grant.delete audit row for {email}");
}

#[tokio::test]
async fn grant_validation_and_revoke_edges() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping grants test");
        return;
    };
    let app = make_app(&pool);
    let admin = testkit::token(testkit::SUB_ADMIN);

    for bad in [
        json!({"subjectType": "sid", "subject": "x", "role": "viewer"}),
        json!({"subjectType": "email", "subject": "", "role": "viewer"}),
        json!({"subjectType": "email", "subject": "not-an-email", "role": "viewer"}),
        json!({"subjectType": "email", "subject": "x@y", "role": "superuser"}),
        json!({"subjectType": "aud", "subject": " ", "role": "viewer"}),
    ] {
        let (status, body) = call(&app, "PUT", "/api/admin/grants", Some(&admin), Some(bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    // Revoking a missing id is a 404.
    let (status, _) = call(
        &app,
        "DELETE",
        "/api/admin/grants/99999999",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_cannot_self_revoke() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping grants test");
        return;
    };
    let app = make_app(&pool);
    let admin = testkit::token(testkit::SUB_ADMIN);

    let (status, body) = call(&app, "GET", "/api/admin/grants", Some(&admin), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = grant_id(&body, "sub", testkit::SUB_ADMIN, "admin").expect("test-admin grant");

    let (status, body) = call(
        &app,
        "DELETE",
        &format!("/api/admin/grants/{id}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // The grant still resolves — admin is untouched.
    let (status, body) = call(&app, "GET", "/api/auth/whoami", Some(&admin), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["roles"].as_array().unwrap().contains(&json!("admin")));
}
