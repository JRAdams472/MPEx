//! Audit trail integration tests (MPE-97 phase 3): attributed, hash-chained
//! rows on every sensitive mutation; tamper detection via `verify_chain`;
//! `auth.denied` rows on 403 while 401 stays log-only.
//! Gated on DATABASE_URL (set in CI); skips locally otherwise.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use tower::ServiceExt;

use mpex::api::{self, AppState};
use mpex::auth::testkit;
use mpex::rules::snapshot::SnapshotStore;

/// Tests that call `verify_chain` serialize on this — the tamper test briefly
/// breaks the chain and parallel verifiers would see it.
static AUDIT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

/// Minimal JDM that survives `DecisionContent` validation.
fn test_jdm() -> Value {
    json!({
        "nodes": [
            {"id": "in", "type": "inputNode", "name": "Request"},
            {"id": "tbl", "type": "decisionTableNode", "name": "Rules",
             "content": {
                "hitPolicy": "first",
                "inputs": [{"id": "i1", "name": "LTV", "field": "loan.ltv"}],
                "outputs": [{"id": "o1", "name": "reason", "field": "reasons[]"}],
                "rules": [{"_id": "r1", "i1": "> 95", "o1": "'ltv exceeds 95%'"}]
             }},
            {"id": "out", "type": "outputNode", "name": "Response"}
        ],
        "edges": [
            {"id": "e1", "sourceId": "in", "targetId": "tbl"},
            {"id": "e2", "sourceId": "tbl", "targetId": "out"}
        ]
    })
}

/// Every artifact mutation writes one attributed chain row; the chain itself
/// verifies end-to-end.
#[tokio::test]
async fn mutations_write_attributed_chained_rows() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping audit test");
        return;
    };
    let _guard = AUDIT_LOCK.lock().await;
    let app = make_app(&pool);
    let admin = testkit::admin_token();
    let key = "audit-test.decision";

    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/decisions/{key}"),
        Some(&admin),
        Some(json!({ "kind": "shared", "jdm": test_jdm() })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let row = sqlx::query(
        "SELECT id, actor_sub, action, artifact_table, artifact_key, detail,
                prev_hash, hash
         FROM audit_events
         WHERE action = 'artifact.put' AND artifact_key = $1
         ORDER BY id DESC LIMIT 1",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .expect("audit row for the put exists");
    assert_eq!(row.get::<String, _>("actor_sub"), testkit::SUB_ADMIN);
    assert_eq!(row.get::<String, _>("artifact_table"), "decisions");
    let hash: String = row.get("hash");
    assert_eq!(hash.len(), 64, "sha256 hex digest");
    // Chained to the tail at insert time (or '' for a genesis row).
    let prev: String = row.get("prev_hash");
    assert!(prev.is_empty() || prev.len() == 64);
    let detail: Value = row.get("detail");
    assert!(
        detail.get("contentHash").and_then(Value::as_str).is_some(),
        "put detail carries a content fingerprint: {detail}"
    );

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/api/decisions/{key}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // `id > put_row.id` isolates this run's writes from prior runs' leftovers.
    let put_row_id: i64 = row.get("id");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_events
         WHERE action = 'artifact.delete' AND artifact_key = $1
           AND actor_sub = $2 AND id > $3",
    )
    .bind(key)
    .bind(testkit::SUB_ADMIN)
    .bind(put_row_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1, "delete audited");

    let verify = mpex::audit::verify_chain(&pool).await.unwrap();
    assert!(verify.ok, "intact chain verifies: {verify:?}");

    sqlx::query("DELETE FROM decisions WHERE key = $1 AND status = 'draft'")
        .bind(key)
        .execute(&pool)
        .await
        .unwrap();
}

/// The publication audit row lands inside the publication tx: a failed
/// publish (empty 409) leaves no audit row.
#[tokio::test]
async fn publish_audit_inside_publication_tx() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping audit test");
        return;
    };
    // Locked: a concurrent PUT from another test would turn the expected 409
    // below into a 201, and a leftover identical draft could 409 the first.
    let _guard = AUDIT_LOCK.lock().await;
    let app = make_app(&pool);
    let admin = testkit::admin_token();
    // Fresh key per run: never published before, so the first publish always
    // has a change to stamp.
    let key = format!("audit-pub.{}", uuid::Uuid::new_v4());
    let label = format!("audit-pub-{}", uuid::Uuid::new_v4());

    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/decisions/{key}"),
        Some(&admin),
        Some(json!({ "kind": "shared", "jdm": test_jdm() })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = call(
        &app,
        "POST",
        "/api/publications",
        Some(&admin),
        Some(json!({ "label": label })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let pub_id = body["id"].as_str().unwrap().to_string();

    let row = sqlx::query(
        "SELECT actor_sub, artifact_table, artifact_key, detail
         FROM audit_events
         WHERE action = 'publication.create' AND artifact_key = $1",
    )
    .bind(&pub_id)
    .fetch_one(&pool)
    .await
    .expect("audit row for the publication exists");
    assert_eq!(row.get::<String, _>("actor_sub"), testkit::SUB_ADMIN);
    assert_eq!(row.get::<String, _>("artifact_table"), "publications");
    assert_eq!(
        row.get::<Value, _>("detail").get("label"),
        Some(&Value::from(label.clone())),
        "publication label attributed"
    );

    // Nothing changed → 409, and the rollback leaves no second audit row.
    let (status, _) = call(
        &app,
        "POST",
        "/api/publications",
        Some(&admin),
        Some(json!({ "label": label })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_events
         WHERE action = 'publication.create' AND detail->>'label' = $1",
    )
    .bind(&label)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1, "rolled-back publish writes no audit row");

    sqlx::query("DELETE FROM decisions WHERE key = $1 AND status = 'draft'")
        .bind(key)
        .execute(&pool)
        .await
        .unwrap();
}

/// A 403 writes an attributed `auth.denied` row; a 401 writes none.
#[tokio::test]
async fn denied_requests_audit_only_403s() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping audit test");
        return;
    };
    let app = make_app(&pool);
    let key = "audit-denied.decision";
    let path = format!("/api/decisions/{key}");

    let (status, _) = call(&app, "PUT", &path, Some(&testkit::viewer_token()), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let row = sqlx::query(
        "SELECT actor_sub, detail FROM audit_events
         WHERE action = 'auth.denied' AND actor_sub = $1
           AND detail->>'path' = $2
         ORDER BY id DESC LIMIT 1",
    )
    .bind(testkit::SUB_VIEWER)
    .bind(&path)
    .fetch_one(&pool)
    .await
    .expect("auth.denied row for the 403 exists");
    assert_eq!(row.get::<Value, _>("detail")["method"], json!("PUT"));

    // Unauthenticated (401) requests are log-only — audit rows for bad or
    // absent tokens would let anyone grow the table unbounded.
    let before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE detail->>'path' = $1")
            .bind(&path)
            .fetch_one(&pool)
            .await
            .unwrap();
    let (status, _) = call(&app, "PUT", &path, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(&app, "PUT", &path, Some("not-a-jwt"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE detail->>'path' = $1")
            .bind(&path)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(after, before, "401s write no audit rows");
}

/// Intact chain verifies; a row rewritten out-of-band (trigger bypassed, as a
/// DBA or attacker with table-owner rights could) breaks verification at the
/// tampered row — then verifies clean again once restored.
#[tokio::test]
async fn verify_chain_detects_tamper() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping audit test");
        return;
    };
    let _guard = AUDIT_LOCK.lock().await;
    let app = make_app(&pool);

    // Ensure the chain has a tail to tamper with.
    let (status, _) = call(
        &app,
        "PUT",
        "/api/decisions/audit-tamper.decision",
        Some(&testkit::admin_token()),
        Some(json!({ "kind": "shared", "jdm": test_jdm() })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (id, orig): (i64, String) =
        sqlx::query_as("SELECT id, actor_sub FROM audit_events ORDER BY id DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();

    // The storage layer refuses ordinary edits…
    let err = sqlx::query("UPDATE audit_events SET actor_sub = 'x' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("append-only"),
        "trigger rejects update: {err}"
    );

    // …but someone bypassing the trigger rewrites history; verify catches it.
    sqlx::query("ALTER TABLE audit_events DISABLE TRIGGER audit_events_append_only")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE audit_events SET actor_sub = 'mallory' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE audit_events ENABLE TRIGGER audit_events_append_only")
        .execute(&pool)
        .await
        .unwrap();

    let verify = mpex::audit::verify_chain(&pool).await.unwrap();
    assert!(!verify.ok, "tampered row must fail verification");
    assert_eq!(verify.first_bad_id, Some(id), "flags the tampered row");

    // Restore the row: the recomputed hash matches again, chain is healthy.
    sqlx::query("ALTER TABLE audit_events DISABLE TRIGGER audit_events_append_only")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE audit_events SET actor_sub = $1 WHERE id = $2")
        .bind(&orig)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE audit_events ENABLE TRIGGER audit_events_append_only")
        .execute(&pool)
        .await
        .unwrap();

    let verify = mpex::audit::verify_chain(&pool).await.unwrap();
    assert!(verify.ok, "restored chain verifies");

    sqlx::query("DELETE FROM decisions WHERE key = 'audit-tamper.decision' AND status = 'draft'")
        .execute(&pool)
        .await
        .unwrap();
}

/// `GET /api/audit` (admin): filtered listing; `GET /api/audit/verify`
/// reports a healthy chain.
#[tokio::test]
async fn audit_endpoints_list_and_verify() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping audit test");
        return;
    };
    let _guard = AUDIT_LOCK.lock().await;
    let app = make_app(&pool);
    let admin = testkit::admin_token();

    let (status, _) = call(
        &app,
        "PUT",
        "/api/decisions/audit-list.decision",
        Some(&admin),
        Some(json!({ "kind": "shared", "jdm": test_jdm() })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = call(
        &app,
        "GET",
        "/api/audit?action=artifact.put&key=audit-list.decision",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let events = body["events"].as_array().unwrap();
    // id DESC order: [0] is this run's row; earlier runs may leave more.
    assert!(!events.is_empty(), "filtered list finds the put row");
    assert_eq!(events[0]["actorSub"], json!(testkit::SUB_ADMIN));
    assert_eq!(events[0]["artifactTable"], json!("decisions"));

    let (status, body) = call(&app, "GET", "/api/audit/verify", Some(&admin), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], json!(true), "{body}");

    sqlx::query("DELETE FROM decisions WHERE key = 'audit-list.decision' AND status = 'draft'")
        .execute(&pool)
        .await
        .unwrap();
}
