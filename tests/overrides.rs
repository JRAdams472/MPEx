//! Value overrides (MPE-126): draft/published override rows patch a target
//! artifact's resolved content at snapshot-build time. Covers PUT
//! validation, draft application + delete-restore, publish/asOf semantics,
//! decision-JDM and matrix-spec patching, and the audit vocabulary.
//! Gated on DATABASE_URL; skips locally otherwise.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

use mpex::api::{self, AppState};
use mpex::auth::testkit;
use mpex::rules::loader::{Env, Revision};
use mpex::rules::snapshot::SnapshotStore;

/// The dev working set is shared — a publish sweeps every draft, so tests
/// that put/publish/delete serialize on this to keep their assertions clean.
static OVR_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const PRODUCT: &str = "OVRTEST";
const DECISION_KEY: &str = "eligibility.OVRTEST";
const EXPR_KEY: &str = "expr.ovr.ok";
const MATRIX_KEY: &str = "llpa.ovr.test";
const OVR_KEY: &str = "ovr.test.flip";

fn make_app(pool: &PgPool) -> (Router, Arc<SnapshotStore>) {
    let snapshots = Arc::new(SnapshotStore::new(pool.clone()));
    (
        api::router(AppState {
            pool: pool.clone(),
            snapshots: snapshots.clone(),
            auth: Some(testkit::test_context(pool.clone())),
            rate_limits: None,
            rate_index: None,
        }),
        snapshots,
    )
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

/// Eligibility decision delegating to the expression via decisionNode; the
/// verdict is just `result`, so patching the expression flips the product.
fn decision_jdm() -> Value {
    json!({
        "nodes": [
            {"id": "in", "type": "inputNode", "name": "Request"},
            {"id": "sub", "type": "decisionNode", "name": "Gate",
             "content": {"key": EXPR_KEY}},
            {"id": "fin", "type": "expressionNode", "name": "Verdict",
             "content": {"expressions": [
                {"id": "e1", "key": "eligible", "value": "result"},
                {"id": "e2", "key": "reasons", "value": "result ? [] : ['gate']"}
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

/// Product eligibility at a revision; `None` = latest published + dev
/// draft-fill, `"draft"` = the draft working set, RFC3339 = asOf.
async fn eligible_at(app: &Router, as_of: Option<&str>) -> bool {
    let mut req = json!({ "loan": { "loan": { "ltv": 75 } } });
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

async fn put_override(
    app: &Router,
    key: &str,
    target_kind: &str,
    target_key: &str,
    value_path: &str,
    value: Value,
) -> (StatusCode, Value) {
    call(
        app,
        "PUT",
        &format!("/api/overrides/{key}"),
        Some(json!({
            "targetKind": target_kind,
            "targetKey": target_key,
            "valuePath": value_path,
            "value": value,
        })),
    )
    .await
}

async fn seed_expr_and_decision(app: &Router) {
    let (s, b) = call(
        app,
        "PUT",
        &format!("/api/expressions/{EXPR_KEY}"),
        Some(json!({ "kind": "standard", "expression": "true" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = call(
        app,
        "PUT",
        &format!("/api/decisions/{DECISION_KEY}"),
        Some(json!({ "kind": "eligibility", "productCode": PRODUCT, "jdm": decision_jdm() })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

async fn publish(app: &Router, label: &str) -> Value {
    let (status, body) = call(
        app,
        "POST",
        "/api/publications",
        Some(json!({ "label": label })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "publish failed: {body}");
    body
}

async fn cleanup(pool: &PgPool) {
    for table in ["overrides", "decisions", "expressions", "matrices"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE publication_id IN
             (SELECT id FROM publications WHERE label LIKE 'ovr-test-%')"
        ))
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query("DELETE FROM publications WHERE label LIKE 'ovr-test-%'")
        .execute(pool)
        .await
        .unwrap();
    for (table, key) in [
        ("overrides", OVR_KEY),
        ("decisions", DECISION_KEY),
        ("expressions", EXPR_KEY),
        ("matrices", MATRIX_KEY),
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE key = $1"))
            .bind(key)
            .execute(pool)
            .await
            .unwrap();
    }
}

async fn setup() -> Option<(PgPool, Router, Arc<SnapshotStore>)> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    testkit::ensure_grants(&pool).await;
    cleanup(&pool).await;
    sqlx::query(
        "INSERT INTO products (code, name, loan_type, amortization, term_months, active)
         VALUES ($1, 'Override Test', 'conventional', 'fixed', 360, true)
         ON CONFLICT (code) DO UPDATE SET active = true",
    )
    .bind(PRODUCT)
    .execute(&pool)
    .await
    .unwrap();
    let (app, snapshots) = make_app(&pool);
    Some((pool, app, snapshots))
}

/// PUT rejects the whole bad-input family: unknown target kind, missing
/// target, non-pointer path, structural node, non-scalar value.
#[tokio::test]
async fn put_validates_target_and_pointer() {
    let _serial = OVR_LOCK.lock().await;
    let Some((_pool, app, _snap)) = setup().await else {
        eprintln!("DATABASE_URL unset — skipping overrides test");
        return;
    };
    seed_expr_and_decision(&app).await;

    // Unknown target kind.
    let (s, b) = put_override(
        &app,
        OVR_KEY,
        "widget",
        EXPR_KEY,
        "/expression",
        json!("false"),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    // Target that doesn't resolve in the working set.
    let (s, b) = put_override(
        &app,
        OVR_KEY,
        "expression",
        "expr.nope",
        "/expression",
        json!("x"),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    // valuePath must be a JSON Pointer.
    let (s, b) = put_override(
        &app,
        OVR_KEY,
        "expression",
        EXPR_KEY,
        "expression",
        json!("x"),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    // Path that doesn't resolve in the target document.
    let (s, b) = put_override(
        &app,
        OVR_KEY,
        "expression",
        EXPR_KEY,
        "/kind",
        json!("unary"),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    // Non-scalar replacement value.
    let (s, b) = put_override(
        &app,
        OVR_KEY,
        "expression",
        EXPR_KEY,
        "/expression",
        json!({"a": 1}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    // Pointer addressing a structural node (nodes is an array).
    let (s, b) = put_override(
        &app,
        OVR_KEY,
        "decision",
        DECISION_KEY,
        "/nodes",
        json!("x"),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    // A patch that would corrupt the target: expression text that doesn't compile.
    let (s, b) = put_override(
        &app,
        OVR_KEY,
        "expression",
        EXPR_KEY,
        "/expression",
        json!("((("),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
}

/// A draft override applies to the working-set evaluation immediately;
/// deleting the draft restores the original value — the ticket's core flow.
#[tokio::test]
async fn draft_override_applies_and_delete_restores() {
    let _serial = OVR_LOCK.lock().await;
    let Some((pool, app, _snap)) = setup().await else {
        eprintln!("DATABASE_URL unset — skipping overrides test");
        return;
    };
    seed_expr_and_decision(&app).await;
    assert!(eligible_at(&app, None).await, "baseline should be eligible");

    let (s, b) = put_override(
        &app,
        OVR_KEY,
        "expression",
        EXPR_KEY,
        "/expression",
        json!("false"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(
        !eligible_at(&app, None).await,
        "draft override should flip the product to ineligible"
    );

    let (s, _) = call(&app, "DELETE", &format!("/api/overrides/{OVR_KEY}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(
        eligible_at(&app, None).await,
        "deleting the draft restores the original value"
    );

    // Audit trail carries both mutations under the override vocabulary —
    // the trail is append-only across test runs, so assert on the tail.
    let actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_events
         WHERE artifact_key = $1 ORDER BY occurred_at DESC LIMIT 2",
    )
    .bind(OVR_KEY)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(actions, ["override.delete", "override.put"]);
}

/// Published overrides apply to published resolution and asOf evaluation;
/// an asOf before the publish still sees the unpatched artifact.
#[tokio::test]
async fn published_override_applies_and_asof_ignores() {
    let _serial = OVR_LOCK.lock().await;
    let Some((_pool, app, _snap)) = setup().await else {
        eprintln!("DATABASE_URL unset — skipping overrides test");
        return;
    };
    seed_expr_and_decision(&app).await;
    let pub1 = publish(&app, "ovr-test-base").await;
    let pub1_at = pub1["publishedAt"].as_str().unwrap().to_string();
    assert!(eligible_at(&app, Some(&pub1_at)).await);

    let (s, b) = put_override(
        &app,
        OVR_KEY,
        "expression",
        EXPR_KEY,
        "/expression",
        json!("false"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let pub2 = publish(&app, "ovr-test-override").await;
    assert_eq!(
        pub2["overrides"].as_array().unwrap().len(),
        1,
        "publication reports the stamped override: {pub2}"
    );

    assert!(
        !eligible_at(&app, None).await,
        "published override applies to latest-published resolution"
    );
    assert!(
        eligible_at(&app, Some(&pub1_at)).await,
        "asOf before the override publish resolves the unpatched expression"
    );
}

/// Decision-JDM patching: a pointer into the graph's expression node flips
/// the verdict directly, no expression indirection involved.
#[tokio::test]
async fn override_patches_decision_jdm() {
    let _serial = OVR_LOCK.lock().await;
    let Some((_pool, app, _snap)) = setup().await else {
        eprintln!("DATABASE_URL unset — skipping overrides test");
        return;
    };
    seed_expr_and_decision(&app).await;
    assert!(eligible_at(&app, None).await);

    // nodes[2] is the verdict expressionNode; expressions[0] computes eligible.
    let (s, b) = put_override(
        &app,
        OVR_KEY,
        "decision",
        DECISION_KEY,
        "/nodes/2/content/expressions/0/value",
        json!("false"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(
        !eligible_at(&app, None).await,
        "patched verdict should make the product ineligible"
    );
}

/// Data-artifact patching: rate-sheet spec leaves patch straight into the
/// snapshot map; matrix specs patch then re-derive loader JDM.
#[tokio::test]
async fn override_patches_rate_sheet_and_matrix_specs() {
    let _serial = OVR_LOCK.lock().await;
    let Some((pool, app, snapshots)) = setup().await else {
        eprintln!("DATABASE_URL unset — skipping overrides test");
        return;
    };

    let (s, b) = call(
        &app,
        "PUT",
        &format!("/api/rate-sheets/{PRODUCT}"),
        Some(json!({
            "spec": {
                "rates": [6.5],
                "lockTerms": [30],
                "prices": { "6.5:30": 100.0 }
            }
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let (s, b) = call(
        &app,
        "PUT",
        &format!("/api/matrices/{MATRIX_KEY}"),
        Some(json!({
            "kind": "llpa",
            "spec": {
                "rows": [{"id": "ltv", "expr": "true"}],
                "cols": [{"id": "band", "expr": "true"}],
                "cells": { "ltv:band": 0.5 }
            }
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let (s, b) = put_override(
        &app,
        "ovr.test.sheet",
        "rate_sheet",
        PRODUCT,
        "/prices/6.5:30",
        json!(99.5),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = put_override(
        &app,
        "ovr.test.matrix",
        "matrix",
        MATRIX_KEY,
        "/cells/ltv:band",
        json!(0.125),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let snap = snapshots
        .snapshot_for(Revision::Draft, Env::Dev)
        .await
        .expect("snapshot builds");
    assert_eq!(
        snap.rate_sheets[PRODUCT]["prices"]["6.5:30"],
        json!(99.5),
        "rate-sheet price leaf patched in the resolved spec"
    );

    // The matrix's loader JDM re-derives from the patched spec — the compiled
    // rule output carries the new cell value (to_jdm serializes cells into
    // "o1" strings).
    let loader = snap.loader.clone();
    let content = zen_engine::loader::DecisionLoader::load(&*loader, MATRIX_KEY)
        .await
        .expect("matrix resolves in loader");
    let jdm_json = serde_json::to_string(&*content).unwrap();
    assert!(
        jdm_json.contains("0.125"),
        "patched matrix cell should appear in loader JDM: {jdm_json}"
    );
    assert!(
        !jdm_json.contains("0.5\""),
        "original cell value should be gone: {jdm_json}"
    );

    sqlx::query("DELETE FROM rate_sheets WHERE key = $1")
        .bind(PRODUCT)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM overrides WHERE key IN ('ovr.test.sheet', 'ovr.test.matrix')")
        .execute(&pool)
        .await
        .unwrap();
}
