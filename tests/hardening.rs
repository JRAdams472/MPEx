//! Hardening checks for the eval path, gated on DATABASE_URL:
//!
//! * determinism — the same request evaluated against a *fresh* snapshot
//!   store (no shared memo) returns an identical body modulo timing fields;
//!   a repeat on the same store is byte-identical (memoized).
//! * concurrency — a burst of parallel requests on shared app state all
//!   return identical bodies and statuses.

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

/// Serializes each test's full body: cleanup → draft PUTs → publish → eval
/// calls. Publishing is global (stamps every changed draft) and the snapshot
/// fingerprint covers ALL artifact tables, so one test's DELETEs mid-run
/// would invalidate the other's memoized fingerprint and break the
/// byte-identical repeat assertion — the tests can't safely overlap.
static SEED_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Per-test fixture namespaces — the two tests run on parallel threads, so
/// each gets its own product/matrix keys and publication label. Sharing them
/// would race (e.g. an identical re-publish → 409).
struct Fixture {
    product: &'static str,
    matrix: &'static str,
    label: &'static str,
}

const DET: Fixture = Fixture {
    product: "P7DTEST",
    matrix: "matrix.p7det.llpa",
    label: "p7-det",
};
const CON: Fixture = Fixture {
    product: "P7CTEST",
    matrix: "matrix.p7con.llpa",
    label: "p7-con",
};

fn make_app(pool: &PgPool) -> Router {
    api::router(AppState {
        pool: pool.clone(),
        snapshots: Arc::new(SnapshotStore::new(pool.clone())),
        auth: Some(testkit::test_context(pool.clone())),
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

fn loan() -> Value {
    json!({
        "loan": { "type": "conventional", "ltv": 81.0, "termMonths": 360 },
        "borrower": { "fico": 760 },
        "scenario": { "lockTermDays": 30 }
    })
}

fn pricing_request(fx: &Fixture) -> Value {
    json!({ "loan": loan(), "products": [fx.product] })
}

/// Timing fields legitimately differ between independently-computed runs
/// (`elapsedMs`, and per-node `performance` strings in eval output).
fn strip_timing(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, x)| {
                    (
                        k.clone(),
                        if k == "elapsedMs" || k == "performance" {
                            json!(0)
                        } else {
                            strip_timing(x)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(strip_timing).collect()),
        other => other.clone(),
    }
}

/// Creates the product/sheet/matrix fixtures and publishes them.
/// The product is inactive: eligibility evaluates only active products, so
/// fixtures can't make a concurrent eligibility burst's product set diverge.
/// Pricing still accepts the code via `products[]` (no active filter there).
async fn seed(pool: &PgPool, fx: &Fixture) -> Router {
    sqlx::query(
        "INSERT INTO products (code, name, loan_type, amortization, term_months, active)
         VALUES ($1, 'P7 Hardening', 'conventional', 'fixed', 360, false)
         ON CONFLICT (code) DO UPDATE SET active = false",
    )
    .bind(fx.product)
    .execute(pool)
    .await
    .unwrap();

    let app = make_app(pool);
    call(
        &app,
        "PUT",
        &format!("/api/rate-sheets/{}", fx.product),
        Some(json!({
            "spec": {
                "rates": [6.5, 6.75],
                "lockTerms": [30],
                "prices": { "6.5:30": 100.0, "6.75:30": 99.5 }
            }
        })),
    )
    .await;
    call(
        &app,
        "PUT",
        &format!("/api/matrices/{}", fx.matrix),
        // productCode scopes the matrix to this fixture's product — a shared
        // (null) matrix would attach to the sibling test's pricing response
        // and make its output diverge mid-run.
        Some(json!({
            "productCode": fx.product,
            "spec": {
                "rows": [{"id": "hi", "expr": "loan.ltv > 80"}],
                "cols": [{"id": "fico", "expr": "borrower.fico >= 740"}],
                "cells": { "hi:fico": -0.25 },
                "op": "sum"
            }
        })),
    )
    .await;
    let (status, body) = call(
        &app,
        "POST",
        "/api/publications",
        Some(json!({ "label": fx.label })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "publish failed: {body}");
    app
}

async fn cleanup(pool: &PgPool, fx: &Fixture) {
    for (table, key) in [("matrices", fx.matrix), ("rate_sheets", fx.product)] {
        sqlx::query(&format!("DELETE FROM {table} WHERE key = $1"))
            .bind(key)
            .execute(pool)
            .await
            .unwrap();
    }
    for table in ["decisions", "expressions", "matrices", "rate_sheets"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE publication_id IN
             (SELECT id FROM publications WHERE label = $1)"
        ))
        .bind(fx.label)
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query("DELETE FROM publications WHERE label = $1")
        .bind(fx.label)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM products WHERE code = $1")
        .bind(fx.product)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn determinism_and_memoization() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping hardening test");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    testkit::ensure_grants(&pool).await;
    let _guard = SEED_LOCK.lock().await;
    cleanup(&pool, &DET).await;
    let app = seed(&pool, &DET).await;

    let req = pricing_request(&DET);
    let (s1, b1) = call(&app, "POST", "/api/evaluate/pricing", Some(req.clone())).await;
    assert_eq!(s1, StatusCode::OK, "first pricing call: {b1}");

    // Same store, same request → memoized, byte-identical including timing.
    let (s2, b2) = call(&app, "POST", "/api/evaluate/pricing", Some(req.clone())).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(b1, b2, "memoized repeat must be byte-identical");

    // Fresh SnapshotStore (empty memo) → identical body modulo timing. This
    // is the real determinism check: eval depends only on request + snapshot.
    let app2 = make_app(&pool);
    let (s3, b3) = call(&app2, "POST", "/api/evaluate/pricing", Some(req)).await;
    assert_eq!(s3, StatusCode::OK);
    assert_eq!(
        strip_timing(&b1),
        strip_timing(&b3),
        "fresh-store response differs beyond timing"
    );

    // Sanity: the fixture actually priced something.
    assert!(
        !b1["priced"].as_array().unwrap().is_empty(),
        "expected priced lines: {b1}"
    );

    cleanup(&pool, &DET).await;
}

#[tokio::test]
async fn concurrent_evals_are_consistent() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping hardening test");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    testkit::ensure_grants(&pool).await;
    let _guard = SEED_LOCK.lock().await;
    cleanup(&pool, &CON).await;
    let app = seed(&pool, &CON).await;

    // One warm request to settle the snapshot cache, then a burst.
    call(
        &app,
        "POST",
        "/api/evaluate/pricing",
        Some(pricing_request(&CON)),
    )
    .await;

    const N: usize = 48;
    let mut handles = Vec::with_capacity(N);
    for i in 0..N {
        let app = app.clone();
        // Mix pricing and eligibility to exercise both parallel paths.
        let (uri, body) = if i % 3 == 0 {
            ("/api/evaluate/eligibility", json!({ "loan": loan() }))
        } else {
            ("/api/evaluate/pricing", pricing_request(&CON))
        };
        handles.push(tokio::spawn(async move {
            let req = Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .header(
                    "authorization",
                    format!("Bearer {}", testkit::admin_token()),
                )
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap();
            let res = app.oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
            (
                status,
                uri.to_string(),
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }));
    }

    let mut pricing_bodies: Vec<Value> = Vec::new();
    let mut elig_bodies: Vec<Value> = Vec::new();
    for h in handles {
        let (status, uri, body) = h.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{uri} failed: {body}");
        if uri.contains("pricing") {
            pricing_bodies.push(strip_timing(&body));
        } else {
            elig_bodies.push(strip_timing(&body));
        }
    }
    assert!(
        pricing_bodies.windows(2).all(|w| w[0] == w[1]),
        "concurrent pricing responses diverged"
    );
    assert!(
        elig_bodies.windows(2).all(|w| w[0] == w[1]),
        "concurrent eligibility responses diverged"
    );

    cleanup(&pool, &CON).await;
}
