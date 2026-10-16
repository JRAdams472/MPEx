//! Rules persistence + publish lifecycle integration test: draft CRUD,
//! atomic publish under a named publication, and per-kind `asOf` resolution
//! proving two published versions stay separable and drafts never leak.
//!
//! Gated on DATABASE_URL (set in CI); skips locally otherwise.

use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

use mpex::api::{self, AppState};
use mpex::rules::snapshot::SnapshotStore;

const PRODUCT: &str = "P2TEST";
const DECISION_KEY: &str = "eligibility.P2TEST";
const EXPR_KEY: &str = "expr.p2test.ltv";

/// Minimal eligibility JDM: fail when `loan.ltv` crosses `threshold`.
fn eligibility_jdm(fail_expr: &str, reason: &str) -> Value {
    json!({
        "nodes": [
            {"id": "in", "type": "inputNode", "name": "Request"},
            {"id": "tbl", "type": "decisionTableNode", "name": "Rules",
             "content": {
                "hitPolicy": "first",
                "inputs": [{"id": "i1", "name": "LTV", "field": "loan.ltv"}],
                "outputs": [{"id": "o1", "name": "reason", "field": "reasons[]"}],
                "rules": [{"_id": "r1", "i1": fail_expr, "o1": reason}]
             }},
            {"id": "fin", "type": "expressionNode", "name": "Verdict",
             "content": {"expressions": [
                {"id": "e1", "key": "eligible", "value": "len(reasons ?? []) == 0"},
                {"id": "e2", "key": "reasons", "value": "reasons ?? []"}
             ]}},
            {"id": "out", "type": "outputNode", "name": "Response"}
        ],
        "edges": [
            {"id": "e1", "sourceId": "in", "targetId": "tbl"},
            {"id": "e2", "sourceId": "tbl", "targetId": "fin"},
            {"id": "e3", "sourceId": "fin", "targetId": "out"}
        ]
    })
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(match body {
            Some(b) => Body::from(serde_json::to_vec(&b).unwrap()),
            None => Body::empty(),
        })
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Evaluate eligibility for our test product at a revision; returns
/// `eligible` for P2TEST.
async fn eligible_at(app: &Router, as_of: Option<&str>) -> bool {
    let mut req = json!({ "loan": { "loan": { "ltv": 81.0 } } });
    if let Some(ts) = as_of {
        req["asOf"] = json!(ts);
    }
    let (status, body) = call(app, "POST", "/api/evaluate/eligibility", Some(req)).await;
    assert_eq!(status, StatusCode::OK, "evaluate failed: {body}");
    for bucket in ["eligible", "ineligible"] {
        if let Some(p) = body[bucket]
            .as_array()
            .and_then(|a| a.iter().find(|p| p["product"]["code"] == PRODUCT))
        {
            return bucket == "eligible" && p["eligible"] == json!(true);
        }
    }
    panic!("{PRODUCT} missing from eligibility report: {body}");
}

#[tokio::test]
async fn publish_lifecycle() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping persistence test");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    // Clean slate for this test's artifacts (test DB only — published rows
    // are immutable in production paths). Prior runs may have stamped drafts
    // under test publications, so clear referencing artifact rows before the
    // publications themselves.
    sqlx::query("DELETE FROM decisions WHERE key = $1")
        .bind(DECISION_KEY)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM expressions WHERE key = $1")
        .bind(EXPR_KEY)
        .execute(&pool)
        .await
        .unwrap();
    for table in ["decisions", "expressions"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE publication_id IN
             (SELECT id FROM publications WHERE label LIKE 'p2-test-%')"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("DELETE FROM publications WHERE label LIKE 'p2-test-%'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO products (code, name, loan_type, amortization, term_months)
         VALUES ($1, 'P2 Test', 'conventional', 'fixed', 360)
         ON CONFLICT (code) DO NOTHING",
    )
    .bind(PRODUCT)
    .execute(&pool)
    .await
    .unwrap();

    let app = api::router(AppState {
        pool: pool.clone(),
        snapshots: Arc::new(SnapshotStore::new(pool.clone())),
    });

    // --- Decision CRUD ------------------------------------------------------
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/decisions/{DECISION_KEY}"),
        Some(json!({
            "kind": "eligibility",
            "productCode": PRODUCT,
            "jdm": eligibility_jdm("> 80", "'v1: ltv exceeds 80%'"),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT decision failed: {body}");
    assert_eq!(body["status"], "draft");

    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/decisions/{DECISION_KEY}"),
        Some(json!({ "jdm": { "bogus": true } })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "invalid JDM accepted");

    // --- Expression CRUD ----------------------------------------------------
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/expressions/{EXPR_KEY}"),
        Some(json!({ "expression": "loan.ltv > 80", "description": "p2 test" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT expression failed: {body}");

    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/expressions/{EXPR_KEY}"),
        Some(json!({ "expression": "loan.ltv >" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "invalid ZEN accepted");

    // --- Publish v1 ---------------------------------------------------------
    let (status, pub_v1) = call(
        &app,
        "POST",
        "/api/publications",
        Some(json!({ "label": "p2-test-v1" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "publish v1 failed: {pub_v1}");
    let t1 = pub_v1["publishedAt"].as_str().unwrap().to_string();
    assert!(pub_v1["decisions"]
        .as_array()
        .unwrap()
        .contains(&json!(DECISION_KEY)));
    assert!(pub_v1["expressions"]
        .as_array()
        .unwrap()
        .contains(&json!(EXPR_KEY)));

    // Draft survives publish (stays editable), published row + history exist.
    let (status, detail) = call(&app, "GET", &format!("/api/decisions/{DECISION_KEY}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(detail["draft"].is_object(), "draft lost after publish");
    assert_eq!(detail["published"]["status"], "published");
    assert_eq!(detail["history"].as_array().unwrap().len(), 1);

    // v1: ltv 81 fails the "> 80" rule → ineligible.
    assert!(!eligible_at(&app, Some(&t1)).await);

    // --- Publish v2 (relaxed rule) ------------------------------------------
    tokio::time::sleep(Duration::from_millis(2)).await; // keep published_at distinct
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/decisions/{DECISION_KEY}"),
        Some(json!({
            "kind": "eligibility",
            "productCode": PRODUCT,
            "jdm": eligibility_jdm("> 90", "'v2: ltv exceeds 90%'"),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, pub_v2) = call(
        &app,
        "POST",
        "/api/publications",
        Some(json!({ "label": "p2-test-v2" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "publish v2 failed: {pub_v2}");
    let t2 = pub_v2["publishedAt"].as_str().unwrap().to_string();
    assert_ne!(t1, t2);

    // Acceptance: same request at two asOf timestamps → respective versions.
    assert!(!eligible_at(&app, Some(&t1)).await, "asOf v1 drifted");
    assert!(eligible_at(&app, Some(&t2)).await, "asOf v2 wrong");

    // --- Draft never leaks into published snapshots -------------------------
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/decisions/{DECISION_KEY}"),
        Some(json!({
            "kind": "eligibility",
            "productCode": PRODUCT,
            "jdm": eligibility_jdm("> 70", "'v3 draft: ltv exceeds 70%'"),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // asOf t2 still v2-eligible, and even "latest" ignores the v3 draft
    // because a published row exists for this key.
    assert!(eligible_at(&app, Some(&t2)).await);
    assert!(eligible_at(&app, None).await);

    tokio::time::sleep(Duration::from_millis(2)).await;
    let (status, pub_v3) = call(
        &app,
        "POST",
        "/api/publications",
        Some(json!({ "label": "p2-test-v3" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "publish v3 failed: {pub_v3}");
    assert!(!eligible_at(&app, None).await, "v3 publish not live");
    // Only the decision changed — the expression was already published.
    assert!(pub_v3["expressions"].as_array().unwrap().is_empty());

    // --- asOf listing per artifact kind --------------------------------------
    let (status, list) = call(&app, "GET", &format!("/api/decisions?asOf={t1}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(list["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["key"] == DECISION_KEY && d["status"] == "published"));

    // --- Empty publish → 409, no half-created publication --------------------
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/api/decisions/{DECISION_KEY}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/api/expressions/{EXPR_KEY}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // Published rows are untouched by draft deletes.
    let (status, detail) = call(&app, "GET", &format!("/api/decisions/{DECISION_KEY}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(detail["draft"].is_null());
    assert!(detail["published"].is_object());
    assert_eq!(detail["history"].as_array().unwrap().len(), 3);

    let drafts_left: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM decisions WHERE status='draft')
              + (SELECT count(*) FROM expressions WHERE status='draft')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    if drafts_left == 0 {
        let (status, _) = call(
            &app,
            "POST",
            "/api/publications",
            Some(json!({ "label": "p2-test-empty" })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "empty publish should 409");
        let (status, pubs) = call(&app, "GET", "/api/publications", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!pubs["publications"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["label"] == "p2-test-empty"));
    }

    // --- Publication detail ---------------------------------------------------
    let pub_id = pub_v1["id"].as_str().unwrap();
    let (status, detail) = call(&app, "GET", &format!("/api/publications/{pub_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["label"], "p2-test-v1");
    assert!(detail["decisions"]
        .as_array()
        .unwrap()
        .contains(&json!(DECISION_KEY)));

    // End-of-test cleanup — don't leak fixtures into the dev catalog.
    sqlx::query("DELETE FROM decisions WHERE key = $1")
        .bind(DECISION_KEY)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM expressions WHERE key = $1")
        .bind(EXPR_KEY)
        .execute(&pool)
        .await
        .unwrap();
    for table in ["decisions", "expressions"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE publication_id IN
             (SELECT id FROM publications WHERE label LIKE 'p2-test-%')"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("DELETE FROM publications WHERE label LIKE 'p2-test-%'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM products WHERE code = $1")
        .bind(PRODUCT)
        .execute(&pool)
        .await
        .unwrap();
}
