//! Expressions-in-snapshot test: published expressions resolve into the
//! eval loader as synthetic decisions consumable via `decisionNode`, and a
//! republished expression invalidates the snapshot fingerprint.
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
use mpex::auth::testkit;
use mpex::rules::snapshot::SnapshotStore;

const PRODUCT: &str = "P8ETEST";
const DECISION_KEY: &str = "eligibility.P8ETEST";
const EXPR_STD: &str = "expr.p8e.band";
const EXPR_UNARY: &str = "expr.p8e.ltvok";

/// Eligibility decision that delegates to a standard expression via
/// decisionNode: the sub-decision emits `{result}`, which merges into
/// context for the verdict node.
fn std_expr_decision_jdm() -> Value {
    json!({
        "nodes": [
            {"id": "in", "type": "inputNode", "name": "Request"},
            {"id": "sub", "type": "decisionNode", "name": "FICO band",
             "content": {"key": EXPR_STD}},
            {"id": "fin", "type": "expressionNode", "name": "Verdict",
             "content": {"expressions": [
                {"id": "e1", "key": "eligible", "value": "result == 'prime'"},
                {"id": "e2", "key": "reasons", "value": "result == 'prime' ? [] : ['fico band not prime']"}
             ]}},
            {"id": "out", "type": "outputNode", "name": "Response"}
        ],
        "edges": [
            {"id": "e1", "sourceId": "in", "targetId": "sub"},
            {"id": "e2", "sourceId": "sub", "targetId": "fin"},
            {"id": "e3", "sourceId": "fin", "targetId": "out"}
        ]
    })
}

/// Same shape for a unary expression: upstream node sets `input`, the
/// unary sub-decision emits `{result: bool}`.
fn unary_expr_decision_jdm() -> Value {
    json!({
        "nodes": [
            {"id": "in", "type": "inputNode", "name": "Request"},
            {"id": "pre", "type": "expressionNode", "name": "Subject",
             "content": {"expressions": [
                {"id": "e1", "key": "input", "value": "loan.ltv"}
             ]}},
            {"id": "sub", "type": "decisionNode", "name": "LTV check",
             "content": {"key": EXPR_UNARY}},
            {"id": "fin", "type": "expressionNode", "name": "Verdict",
             "content": {"expressions": [
                {"id": "e1", "key": "eligible", "value": "result"},
                {"id": "e2", "key": "reasons", "value": "result ? [] : ['ltv check failed']"}
             ]}},
            {"id": "out", "type": "outputNode", "name": "Response"}
        ],
        "edges": [
            {"id": "e1", "sourceId": "in", "targetId": "pre"},
            {"id": "e2", "sourceId": "pre", "targetId": "sub"},
            {"id": "e3", "sourceId": "sub", "targetId": "fin"},
            {"id": "e4", "sourceId": "fin", "targetId": "out"}
        ]
    })
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header(
            "authorization",
            format!("Bearer {}", testkit::admin_token()),
        )
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

async fn eligible(app: &Router, ltv: f64) -> bool {
    let (status, body) = call(
        app,
        "POST",
        "/api/evaluate/eligibility",
        Some(json!({ "loan": { "loan": { "ltv": ltv }, "borrower": { "fico": 760 } } })),
    )
    .await;
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

async fn cleanup(pool: &PgPool) {
    sqlx::query("DELETE FROM decisions WHERE key = $1")
        .bind(DECISION_KEY)
        .execute(pool)
        .await
        .unwrap();
    for key in [EXPR_STD, EXPR_UNARY] {
        sqlx::query("DELETE FROM expressions WHERE key = $1")
            .bind(key)
            .execute(pool)
            .await
            .unwrap();
    }
    for table in ["decisions", "expressions"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE publication_id IN
             (SELECT id FROM publications WHERE label LIKE 'p8e-test-%')"
        ))
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query("DELETE FROM publications WHERE label LIKE 'p8e-test-%'")
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn expressions_resolve_into_eval_snapshot() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping expressions test");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    testkit::ensure_grants(&pool).await;
    cleanup(&pool).await;

    sqlx::query(
        "INSERT INTO products (code, name, loan_type, amortization, term_months, active)
         VALUES ($1, 'P8 Expr Test', 'conventional', 'fixed', 360, true)
         ON CONFLICT (code) DO UPDATE SET active = true",
    )
    .bind(PRODUCT)
    .execute(&pool)
    .await
    .unwrap();

    let app = api::router(AppState {
        pool: pool.clone(),
        snapshots: Arc::new(SnapshotStore::new(pool.clone())),
        auth: Some(testkit::test_context(pool.clone())),
    });

    // standard expression: fico band string
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/expressions/{EXPR_STD}"),
        Some(json!({
            "kind": "standard",
            "expression": "borrower.fico >= 740 ? 'prime' : 'near-prime'"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT std expression failed: {body}");

    // unary expression: LTV predicate evaluated against `input`
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/expressions/{EXPR_UNARY}"),
        Some(json!({ "kind": "unary", "expression": "<= 80" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "PUT unary expression failed: {body}"
    );

    // --- standard expression via decisionNode -------------------------------
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/decisions/{DECISION_KEY}"),
        Some(json!({
            "kind": "eligibility",
            "productCode": PRODUCT,
            "jdm": std_expr_decision_jdm(),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT decision failed: {body}");

    let (status, pub_v1) = call(
        &app,
        "POST",
        "/api/publications",
        Some(json!({ "label": "p8e-test-v1" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "publish v1 failed: {pub_v1}");
    assert!(pub_v1["expressions"]
        .as_array()
        .unwrap()
        .contains(&json!(EXPR_STD)));

    // fico 760 -> 'prime' -> eligible
    assert!(eligible(&app, 81.0).await, "standard expr not consulted");

    // --- unary expression via decisionNode ----------------------------------
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/decisions/{DECISION_KEY}"),
        Some(json!({
            "kind": "eligibility",
            "productCode": PRODUCT,
            "jdm": unary_expr_decision_jdm(),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT unary decision failed: {body}");

    let (status, pub_v2) = call(
        &app,
        "POST",
        "/api/publications",
        Some(json!({ "label": "p8e-test-v2" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "publish v2 failed: {pub_v2}");

    assert!(
        eligible(&app, 75.0).await,
        "unary expr should pass at ltv 75"
    );
    assert!(
        !eligible(&app, 85.0).await,
        "unary expr should fail at ltv 85"
    );

    // --- republish bumps the snapshot fingerprint ----------------------------
    tokio::time::sleep(Duration::from_millis(2)).await;
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/expressions/{EXPR_UNARY}"),
        Some(json!({ "kind": "unary", "expression": "<= 90" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT unary v2 failed: {body}");

    let (status, pub_v3) = call(
        &app,
        "POST",
        "/api/publications",
        Some(json!({ "label": "p8e-test-v3" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "publish v3 failed: {pub_v3}");
    assert!(pub_v3["expressions"]
        .as_array()
        .unwrap()
        .contains(&json!(EXPR_UNARY)));

    // ltv 85 now passes — the new expression text resolved without staleness.
    assert!(
        eligible(&app, 85.0).await,
        "republished expression not live"
    );

    cleanup(&pool).await;
    sqlx::query("DELETE FROM products WHERE code = $1")
        .bind(PRODUCT)
        .execute(&pool)
        .await
        .unwrap();
}
