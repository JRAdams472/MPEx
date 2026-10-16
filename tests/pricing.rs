//! Pricing pipeline tests: rate-sheet spec validation runs everywhere; the
//! end-to-end lifecycle (draft → publish → price at two asOf revisions,
//! products filter, memoization) is gated on DATABASE_URL.

use serde_json::{json, Value};

use mpex::rules::rate_sheet::RateSheetSpec;

fn demo_spec() -> Value {
    json!({
        "rates": [6.5, 6.75],
        "lockTerms": [30, 45],
        "prices": {
            "6.5:30": 100.0,
            "6.5:45": 99.75,
            "6.75:30": 99.5,
            "6.75:45": 99.25
        }
    })
}

#[test]
fn spec_parses_and_looks_up_prices() {
    let spec = RateSheetSpec::parse(&demo_spec()).unwrap();
    assert_eq!(spec.price(6.5, 30), Some(100.0));
    assert_eq!(spec.price(6.75, 45), Some(99.25));
    assert_eq!(spec.price(6.5, 60), None, "unlisted term");
    assert_eq!(spec.price(7.0, 30), None, "unlisted rate");
    // "6.50" and 6.5 are the same grid point.
    assert_eq!(spec.price(6.50, 30), Some(100.0));
    assert_eq!(spec.grid().len(), 4);
}

#[test]
fn spec_validation() {
    let mut bad = demo_spec();
    bad["rates"] = json!([]);
    assert!(RateSheetSpec::parse(&bad).is_err(), "empty rates must fail");

    let mut bad = demo_spec();
    bad["prices"]["9.9:30"] = json!(100.0);
    assert!(
        RateSheetSpec::parse(&bad).is_err(),
        "unlisted rate must fail"
    );

    let mut bad = demo_spec();
    bad["prices"]["6.5:7"] = json!(100.0);
    assert!(
        RateSheetSpec::parse(&bad).is_err(),
        "unlisted term must fail"
    );

    let mut bad = demo_spec();
    bad["prices"]["6.5:30"] = json!("par");
    assert!(
        RateSheetSpec::parse(&bad).is_err(),
        "non-numeric price must fail"
    );

    let mut bad = demo_spec();
    bad["prices"]["junk"] = json!(100.0);
    assert!(
        RateSheetSpec::parse(&bad).is_err(),
        "bad key shape must fail"
    );

    let mut bad = demo_spec();
    bad["rates"] = json!([6.5, 6.50]);
    assert!(RateSheetSpec::parse(&bad).is_err(), "dup rate must fail");
}

// ---------------------------------------------------------------------------
// DB-gated lifecycle.
// ---------------------------------------------------------------------------

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use sqlx::PgPool;
use tower::ServiceExt;

use mpex::api::{self, AppState};
use mpex::rules::snapshot::SnapshotStore;

const PRODUCT: &str = "P4TEST";
const MATRIX_KEY: &str = "matrix.p4test.llpa";
const SHARED_KEY: &str = "matrix.p4test.shared";

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

fn loan() -> Value {
    json!({
        "loan": { "ltv": 81.0 },
        "borrower": { "fico": 760 },
        "scenario": { "lockTermDays": 30 }
    })
}

/// Shared LTV/FICO matrix: cells keyed on `loan.rate` prove pricing inputs
/// are injected into the eval context. `value` on the 6.5 line only.
fn llpa_spec(adj: f64) -> Value {
    json!({
        "rows": [{"id": "hi", "expr": "loan.ltv > 80"}],
        "cols": [{"id": "r65", "expr": "loan.rate == 6.5"}],
        "cells": { "hi:r65": adj },
        "op": "sum"
    })
}

/// Shared matrix (no productCode) that hits every priced line.
fn shared_spec(adj: f64) -> Value {
    json!({
        "rows": [{"id": "all", "expr": "loan.ltv > 0"}],
        "cols": [{"id": "all", "expr": "borrower.fico > 0"}],
        "cells": { "all:all": adj },
        "op": "sum"
    })
}

fn sheet_spec(price65: f64) -> Value {
    json!({
        "rates": [6.5, 6.75],
        "lockTerms": [30],
        "prices": { "6.5:30": price65, "6.75:30": 99.5 }
    })
}

async fn price(app: &Router, req: Value) -> (StatusCode, Value) {
    call(app, "POST", "/api/evaluate/pricing", Some(req)).await
}

#[tokio::test]
async fn pricing_lifecycle() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping pricing lifecycle test");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    // Clean slate for this test's artifacts.
    for (table, key) in [
        ("matrices", MATRIX_KEY),
        ("matrices", SHARED_KEY),
        ("rate_sheets", PRODUCT),
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE key = $1"))
            .bind(key)
            .execute(&pool)
            .await
            .unwrap();
    }
    for table in ["decisions", "expressions", "matrices", "rate_sheets"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE publication_id IN
             (SELECT id FROM publications WHERE label LIKE 'p4-test-%')"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("DELETE FROM publications WHERE label LIKE 'p4-test-%'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO products (code, name, loan_type, amortization, term_months)
         VALUES ($1, 'P4 Test', 'conventional', 'fixed', 360)
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

    // --- Rate sheet CRUD + validation ---------------------------------------
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/rate-sheets/{PRODUCT}"),
        Some(json!({ "spec": sheet_spec(100.0) })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT rate sheet failed: {body}");
    assert_eq!(body["status"], "draft");

    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/rate-sheets/{PRODUCT}"),
        Some(json!({ "spec": { "rates": [6.5], "lockTerms": [30],
                               "prices": {"7.0:30": 99.0} } })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "bad spec accepted");

    let (status, _) = call(
        &app,
        "PUT",
        "/api/rate-sheets/NOSUCHPRODUCT",
        Some(json!({ "spec": sheet_spec(100.0) })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "unknown product accepted");

    // --- Matrices: product-specific + shared --------------------------------
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/matrices/{MATRIX_KEY}"),
        Some(json!({ "kind": "llpa", "productCode": PRODUCT, "spec": llpa_spec(0.25) })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/matrices/{SHARED_KEY}"),
        Some(json!({ "kind": "llpa", "spec": shared_spec(-0.125) })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // --- Publish v1 → price --------------------------------------------------
    let (status, pub_v1) = call(
        &app,
        "POST",
        "/api/publications",
        Some(json!({ "label": "p4-test-v1" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "publish v1 failed: {pub_v1}");
    assert!(pub_v1["rateSheets"]
        .as_array()
        .unwrap()
        .contains(&json!(PRODUCT)));
    assert!(pub_v1["matrices"]
        .as_array()
        .unwrap()
        .contains(&json!(MATRIX_KEY)));
    let t1 = pub_v1["publishedAt"].as_str().unwrap().to_string();

    // All-rates pricing: two lines, waterfall = product matrix + shared matrix.
    let (status, body) = price(&app, json!({ "loan": loan(), "products": [PRODUCT] })).await;
    assert_eq!(status, StatusCode::OK, "pricing failed: {body}");
    let priced = body["priced"].as_array().unwrap();
    assert_eq!(priced.len(), 1);
    let lines = priced[0]["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 2, "one line per offered rate: {lines:?}");

    // Waterfall values keyed by matrix — the dev DB may carry other shared
    // llpa matrices, so assert our entries plus internal consistency.
    let waterfall_sum = |line: &Value| -> f64 {
        line["adjustments"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|a| a["value"].as_f64())
            .sum()
    };
    let entry = |line: &Value, key: &str| -> Value {
        line["adjustments"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["matrix"] == json!(key))
            .cloned()
            .unwrap_or_else(|| panic!("matrix {key} missing from waterfall: {line}"))
    };

    let l65 = lines.iter().find(|l| l["rate"] == json!(6.5)).unwrap();
    assert_eq!(l65["basePrice"], json!(100.0));
    assert_eq!(entry(l65, MATRIX_KEY)["value"], json!(0.25));
    assert_eq!(entry(l65, SHARED_KEY)["value"], json!(-0.125));
    let total = l65["adjustmentTotal"].as_f64().unwrap();
    assert!((total - waterfall_sum(l65)).abs() < 1e-9);
    assert!((l65["price"].as_f64().unwrap() - (100.0 + total)).abs() < 1e-9);

    let l675 = lines.iter().find(|l| l["rate"] == json!(6.75)).unwrap();
    // rate==6.5 cell misses here → the product matrix contributes 0.
    assert_eq!(entry(l675, MATRIX_KEY)["value"], json!(0.0));
    assert_eq!(entry(l675, SHARED_KEY)["value"], json!(-0.125));
    assert_eq!(l675["basePrice"], json!(99.5));

    // Pinned rate + unknown product in the same request.
    let (status, body) = price(
        &app,
        json!({ "loan": loan(), "products": [PRODUCT, "NOPE"], "rate": 6.5 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "pricing failed: {body}");
    let priced = body["priced"].as_array().unwrap();
    assert_eq!(priced.len(), 1);
    assert_eq!(
        priced[0]["lines"].as_array().unwrap().len(),
        1,
        "rate pinned"
    );
    let unpriced = body["unpriced"].as_array().unwrap();
    assert_eq!(unpriced.len(), 1);
    assert_eq!(unpriced[0]["product"]["code"], json!("NOPE"));

    // Products filter: unlisted products are never priced.
    let (status, body) = price(&app, json!({ "loan": loan(), "products": [] })).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["priced"].as_array().unwrap().is_empty());

    // Memoization: a repeat call returns the identical body (elapsedMs included).
    let req = json!({ "loan": loan(), "products": [PRODUCT], "rate": 6.75 });
    let (_, first) = price(&app, req.clone()).await;
    let (_, second) = price(&app, req).await;
    assert_eq!(first, second, "memoized response should be byte-identical");

    // --- Publish v2 → asOf sees the right revision ---------------------------
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/rate-sheets/{PRODUCT}"),
        Some(json!({ "spec": sheet_spec(99.0) })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/matrices/{MATRIX_KEY}"),
        Some(json!({ "kind": "llpa", "productCode": PRODUCT, "spec": llpa_spec(1.0) })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, pub_v2) = call(
        &app,
        "POST",
        "/api/publications",
        Some(json!({ "label": "p4-test-v2" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "publish v2 failed: {pub_v2}");
    let t2 = pub_v2["publishedAt"].as_str().unwrap().to_string();

    // v2: base 99.0 on the sheet, product matrix now adjusts +1.0.
    let (status, body) = price(
        &app,
        json!({ "loan": loan(), "products": [PRODUCT], "rate": 6.5, "asOf": t2 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "asOf pricing failed: {body}");
    let line = &body["priced"][0]["lines"][0];
    assert_eq!(line["basePrice"], json!(99.0), "v2 sheet expected");
    assert_eq!(entry(line, MATRIX_KEY)["value"], json!(1.0));

    // Same request asOf t1 still prices the v1 artifacts.
    let (status, body) = price(
        &app,
        json!({ "loan": loan(), "products": [PRODUCT], "rate": 6.5, "asOf": t1 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let line = &body["priced"][0]["lines"][0];
    assert_eq!(line["basePrice"], json!(100.0), "v1 sheet expected");
    assert_eq!(entry(line, MATRIX_KEY)["value"], json!(0.25));

    // Draft edits never leak into asOf snapshots.
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/rate-sheets/{PRODUCT}"),
        Some(json!({ "spec": sheet_spec(50.0) })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = price(
        &app,
        json!({ "loan": loan(), "products": [PRODUCT], "rate": 6.5, "asOf": t2 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let line = &body["priced"][0]["lines"][0];
    assert_eq!(line["basePrice"], json!(99.0), "v2 sheet expected");
    assert_eq!(entry(line, MATRIX_KEY)["value"], json!(1.0));

    // Rate sheet detail: draft + published + history.
    let (status, detail) = call(&app, "GET", &format!("/api/rate-sheets/{PRODUCT}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(detail["draft"].is_object());
    assert_eq!(detail["history"].as_array().unwrap().len(), 2);

    // Draft delete leaves published history intact.
    let (status, _) = call(&app, "DELETE", &format!("/api/rate-sheets/{PRODUCT}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, detail) = call(&app, "GET", &format!("/api/rate-sheets/{PRODUCT}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(detail["draft"].is_null());
    assert_eq!(detail["history"].as_array().unwrap().len(), 2);

    // End-of-test cleanup — don't leak fixtures into the dev catalog.
    for (table, key) in [
        ("matrices", MATRIX_KEY),
        ("matrices", SHARED_KEY),
        ("rate_sheets", PRODUCT),
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE key = $1"))
            .bind(key)
            .execute(&pool)
            .await
            .unwrap();
    }
    for table in ["decisions", "expressions", "matrices", "rate_sheets"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE publication_id IN
             (SELECT id FROM publications WHERE label LIKE 'p4-test-%')"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("DELETE FROM publications WHERE label LIKE 'p4-test-%'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM products WHERE code = $1")
        .bind(PRODUCT)
        .execute(&pool)
        .await
        .unwrap();
}
