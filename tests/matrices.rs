//! Matrix tests: spec → JDM compile + collect semantics run everywhere
//! (MemoryLoader, no DB); the publish lifecycle test is gated on DATABASE_URL.

use std::sync::Arc;

use serde_json::{json, Value};
use zen_engine::loader::MemoryLoader;
use zen_engine::model::DecisionContent;
use zen_engine::DecisionEngine;

use mpex::rules::matrix::{self, MatrixSpec};

/// LLPA-ish matrix: LTV rows × FICO cols. At ltv=81/fico=750 rows rA and rB
/// both match (overlap is legal) and only col cX matches → two adjustments.
fn demo_spec() -> Value {
    json!({
        "rows": [
            {"id": "rA", "label": "LTV > 75",  "expr": "loan.ltv > 75"},
            {"id": "rB", "label": "LTV > 80",  "expr": "loan.ltv > 80"},
            {"id": "rC", "label": "LTV > 90",  "expr": "loan.ltv > 90"}
        ],
        "cols": [
            {"id": "cX", "label": "FICO >= 700", "expr": "borrower.creditScore >= 700"},
            {"id": "cY", "label": "FICO >= 780", "expr": "borrower.creditScore >= 780"}
        ],
        "cells": {
            "rA:cX": -0.25,
            "rA:cY": -0.75,
            "rB:cX": 0.5,
            "rC:cX": 1.5
        },
        "op": "sum"
    })
}

fn demo_ctx() -> Value {
    json!({ "loan": { "ltv": 81.0 }, "borrower": { "creditScore": 750 } })
}

#[tokio::test]
async fn spec_compiles_and_collects_overlapping_matches() {
    let spec = MatrixSpec::parse(&demo_spec()).unwrap();
    let jdm = spec.to_jdm().unwrap();

    let loader = MemoryLoader::default();
    loader.add(
        "matrix.test",
        DecisionContent::from(
            serde_json::from_value::<zen_engine::model::GraphContent>(jdm).unwrap(),
        ),
    );
    let engine = DecisionEngine::default().with_loader(Arc::new(loader));

    let res = engine
        .evaluate_serialized("matrix.test", demo_ctx().into(), Default::default())
        .await
        .unwrap();
    let result = &res["result"];
    // rA:cX (-0.25) + rB:cX (0.5) — rC row fails, cY col fails.
    assert_eq!(
        result["adjustments"].as_array().unwrap().len(),
        2,
        "both overlapping rows should collect: {result}"
    );
    let value = result["value"].as_f64().unwrap();
    assert!(
        (value - 0.25).abs() < 1e-9,
        "sum(-0.25, 0.5) = 0.25, got {value}"
    );
    assert_eq!(result["op"], json!("sum"));
}

#[tokio::test]
async fn spec_validation_rejects_bad_references() {
    let mut bad = demo_spec();
    bad["cells"]["nope:cX"] = json!(0.1);
    assert!(MatrixSpec::parse(&bad).is_err(), "unknown row must fail");

    let mut bad = demo_spec();
    bad["rows"][0]["expr"] = json!("loan.ltv >");
    assert!(MatrixSpec::parse(&bad).is_err(), "bad ZEN must fail");

    let mut bad = demo_spec();
    bad["cells"]["rA:cX"] = json!({"nested": true});
    assert!(MatrixSpec::parse(&bad).is_err(), "non-scalar must fail");

    let mut bad = demo_spec();
    bad["op"] = json!("bogus");
    assert!(MatrixSpec::parse(&bad).is_err(), "unknown op must fail");
}

#[tokio::test]
async fn simulate_returns_per_cell_match_detail() {
    let out = matrix::simulate(&demo_spec(), &demo_ctx()).await.unwrap();
    assert_eq!(out.cells.len(), 4, "one entry per populated cell");

    let r_a_cx = out
        .cells
        .iter()
        .find(|c| c.row == "rA" && c.col == "cX")
        .unwrap();
    assert!(r_a_cx.matched && r_a_cx.row_matched && r_a_cx.col_matched);

    let r_a_cy = out
        .cells
        .iter()
        .find(|c| c.row == "rA" && c.col == "cY")
        .unwrap();
    assert!(r_a_cy.row_matched && !r_a_cy.col_matched && !r_a_cy.matched);

    let r_c_cx = out
        .cells
        .iter()
        .find(|c| c.row == "rC" && c.col == "cX")
        .unwrap();
    assert!(!r_c_cx.row_matched && r_c_cx.col_matched && !r_c_cx.matched);

    assert_eq!(out.result["value"].as_f64().unwrap(), 0.25);
}

// ---------------------------------------------------------------------------
// DB-gated lifecycle: draft → publish → snapshot eval at two asOf revisions.
// ---------------------------------------------------------------------------

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use sqlx::PgPool;
use tower::ServiceExt;

use mpex::api::{self, AppState};
use mpex::auth::testkit;
use mpex::rules::loader::{Env, Revision};
use mpex::rules::snapshot::SnapshotStore;

const MATRIX_KEY: &str = "matrix.p3test.llpa";

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

/// Evaluate a decision key through the snapshot loader for a revision.
async fn eval_at(snapshots: &SnapshotStore, revision: Revision, ctx: Value) -> Value {
    let loader = snapshots.loader_for(revision, Env::Dev).await.unwrap();
    let engine = DecisionEngine::default().with_loader(loader);
    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(engine.evaluate_serialized(MATRIX_KEY, ctx.into(), Default::default()))
            .unwrap()
    })
    .await
    .unwrap()
}

fn spec_with_cell(value: f64) -> Value {
    json!({
        "rows": [{"id": "rA", "expr": "loan.ltv > 75"}],
        "cols": [{"id": "cX", "expr": "borrower.creditScore >= 700"}],
        "cells": { "rA:cX": value },
        "op": "sum"
    })
}

#[tokio::test]
async fn matrix_publish_lifecycle() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping matrix lifecycle test");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    testkit::ensure_grants(&pool).await;

    sqlx::query("DELETE FROM matrices WHERE key = $1")
        .bind(MATRIX_KEY)
        .execute(&pool)
        .await
        .unwrap();
    for table in ["decisions", "expressions", "matrices"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE publication_id IN
             (SELECT id FROM publications WHERE label LIKE 'p3-test-%')"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("DELETE FROM publications WHERE label LIKE 'p3-test-%'")
        .execute(&pool)
        .await
        .unwrap();

    let snapshots = Arc::new(SnapshotStore::new(pool.clone()));
    let app = api::router(AppState {
        pool: pool.clone(),
        snapshots: snapshots.clone(),
        auth: Some(testkit::test_context(pool.clone())),
        rate_limits: None,
        rate_index: None,
    });

    // --- Draft CRUD + simulate ----------------------------------------------
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/matrices/{MATRIX_KEY}"),
        Some(json!({ "kind": "llpa", "spec": spec_with_cell(-0.25) })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT matrix failed: {body}");
    assert_eq!(body["status"], "draft");
    assert!(body["jdm"].is_object(), "draft carries compiled jdm");

    let (status, sim) = call(
        &app,
        "POST",
        "/api/simulate/matrix",
        Some(json!({ "spec": spec_with_cell(-0.25), "context": demo_ctx() })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "simulate failed: {sim}");
    assert_eq!(sim["cells"][0]["matched"], json!(true));
    assert_eq!(sim["result"]["value"].as_f64().unwrap(), -0.25);

    // --- Publish v1 → snapshot eval ------------------------------------------
    let (status, pub_v1) = call(
        &app,
        "POST",
        "/api/publications",
        Some(json!({ "label": "p3-test-v1" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "publish v1 failed: {pub_v1}");
    assert!(pub_v1["matrices"]
        .as_array()
        .unwrap()
        .contains(&json!(MATRIX_KEY)));
    let t1 = pub_v1["publishedAt"].as_str().unwrap().to_string();

    let res = eval_at(&snapshots, Revision::LatestPublished, demo_ctx()).await;
    assert_eq!(res["result"]["value"].as_f64().unwrap(), -0.25);

    // --- Publish v2 → two asOf versions stay separable ------------------------
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/matrices/{MATRIX_KEY}"),
        Some(json!({ "kind": "llpa", "spec": spec_with_cell(1.5) })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, pub_v2) = call(
        &app,
        "POST",
        "/api/publications",
        Some(json!({ "label": "p3-test-v2" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "publish v2 failed: {pub_v2}");
    let t2 = pub_v2["publishedAt"].as_str().unwrap().to_string();

    let as_of = |ts: &str| {
        chrono::DateTime::parse_from_rfc3339(ts)
            .unwrap()
            .with_timezone(&chrono::Utc)
    };
    let res = eval_at(&snapshots, Revision::AsOf(as_of(&t1)), demo_ctx()).await;
    assert_eq!(res["result"]["value"].as_f64().unwrap(), -0.25);
    let res = eval_at(&snapshots, Revision::AsOf(as_of(&t2)), demo_ctx()).await;
    assert_eq!(res["result"]["value"].as_f64().unwrap(), 1.5);

    // Draft listing vs published-asOf listing.
    let (status, list) = call(&app, "GET", "/api/matrices", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(list["matrices"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["key"] == MATRIX_KEY && m["status"] == "draft"));
    let (status, list) = call(&app, "GET", &format!("/api/matrices?asOf={t1}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(list["matrices"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["key"] == MATRIX_KEY && m["status"] == "published"));

    // Detail: draft + published + history.
    let (status, detail) = call(&app, "GET", &format!("/api/matrices/{MATRIX_KEY}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(detail["draft"].is_object());
    assert_eq!(detail["history"].as_array().unwrap().len(), 2);

    // Draft deletes leave published history intact.
    let (status, _) = call(&app, "DELETE", &format!("/api/matrices/{MATRIX_KEY}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, detail) = call(&app, "GET", &format!("/api/matrices/{MATRIX_KEY}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(detail["draft"].is_null());
    assert_eq!(detail["history"].as_array().unwrap().len(), 2);

    // End-of-test cleanup — don't leak fixtures into the dev catalog.
    sqlx::query("DELETE FROM matrices WHERE key = $1")
        .bind(MATRIX_KEY)
        .execute(&pool)
        .await
        .unwrap();
    for table in ["decisions", "expressions", "matrices"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE publication_id IN
             (SELECT id FROM publications WHERE label LIKE 'p3-test-%')"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("DELETE FROM publications WHERE label LIKE 'p3-test-%'")
        .execute(&pool)
        .await
        .unwrap();
}
