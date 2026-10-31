//! Edge-hardening integration tests (MPE-97 phase 5): key/label/size
//! validation through AppError, the 2 MiB body cap, and per-IP rate limits
//! (429 + Retry-After, XFF partitioning when trusted).
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
use mpex::limits::EdgeLimits;
use mpex::rules::snapshot::SnapshotStore;

fn make_app(pool: &PgPool) -> Router {
    make_app_with_limits(pool, None)
}

fn make_app_with_limits(pool: &PgPool, limits: Option<Arc<EdgeLimits>>) -> Router {
    api::router(AppState {
        pool: pool.clone(),
        snapshots: Arc::new(SnapshotStore::new(pool.clone())),
        auth: Some(testkit::test_context(pool.clone())),
        rate_limits: limits,
        rate_index: None,
    })
}

fn req(
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
    xff: Option<&str>,
) -> Request<Body> {
    let mut r = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = token {
        r = r.header("authorization", format!("Bearer {t}"));
    }
    if let Some(ip) = xff {
        r = r.header("x-forwarded-for", ip);
    }
    r.body(match body {
        Some(b) => Body::from(serde_json::to_vec(&b).unwrap()),
        None => Body::empty(),
    })
    .unwrap()
}

async fn status(app: &Router, r: Request<Body>) -> StatusCode {
    app.clone().oneshot(r).await.unwrap().status()
}

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = PgPool::connect(&url).await.unwrap();
    testkit::ensure_grants(&pool).await;
    Some(pool)
}

fn jdm() -> Value {
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

/// `{key}` regex + caps: bad keys 400 on every artifact route, valid keys pass.
#[tokio::test]
async fn keys_labels_and_sizes_are_bounded() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping edge test");
        return;
    };
    let app = make_app(&pool);
    let admin = testkit::admin_token();
    let put = |key: &str, body: Value| {
        req(
            "PUT",
            &format!("/api/decisions/{key}"),
            Some(&admin),
            Some(body),
            None,
        )
    };

    // Bad keys → 400: control chars, leading dot, over-length.
    for bad in [
        "bad%20key",
        ".lead",
        "%24whoami",
        &"k".repeat(129),
        "a%2Fb", // decoded '/' would smuggle a path segment
    ] {
        let s = status(&app, put(bad, json!({"kind": "shared", "jdm": jdm()}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "key {bad:?} must be rejected");
    }
    // GET validates too.
    let s = status(
        &app,
        req("GET", "/api/decisions/bad%20key", Some(&admin), None, None),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Control: a clean key is accepted.
    let key = format!("edge-ok.{}", uuid::Uuid::new_v4());
    let s = status(&app, put(&key, json!({"kind": "shared", "jdm": jdm()}))).await;
    assert_eq!(s, StatusCode::OK, "valid key must pass");
    sqlx::query("DELETE FROM decisions WHERE key = $1")
        .bind(&key)
        .execute(&pool)
        .await
        .unwrap();

    // Label cap: 200 ok, 201 rejected before any DB work.
    let s = status(
        &app,
        req(
            "POST",
            "/api/publications",
            Some(&admin),
            Some(json!({ "label": "x".repeat(201) })),
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Expression source cap: 64 KiB.
    let s = status(
        &app,
        req(
            "PUT",
            "/api/expressions/edge-expr",
            Some(&admin),
            Some(json!({ "expression": "x".repeat(70_000) })),
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Spec size cap: 1 MiB serialized.
    let s = status(
        &app,
        put(
            "edge-big.decision",
            json!({ "kind": "shared", "jdm": { "pad": "x".repeat(1_100_000) } }),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

/// The 2 MiB DefaultBodyLimit rejects oversized bodies with 413 before the
/// handler (or its validation) runs.
#[tokio::test]
async fn body_limit_rejects_413() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping edge test");
        return;
    };
    let app = make_app(&pool);
    let admin = testkit::admin_token();
    let s = status(
        &app,
        req(
            "PUT",
            "/api/decisions/edge-huge.decision",
            Some(&admin),
            Some(json!({ "kind": "shared", "jdm": { "pad": "x".repeat(3_000_000) } })),
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
}

/// Per-IP token buckets: the global quota gates everything, the mutation
/// quota gates PUT/DELETE/publish tighter. 429s carry Retry-After.
#[tokio::test]
async fn rate_limits_429_with_retry_after() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping edge test");
        return;
    };
    // global=3, mutations=1 on the shared "unknown" bucket.
    // Applied via AppState exactly as main.rs wires it.
    let app = make_app_with_limits(&pool, Some(Arc::new(EdgeLimits::new(3, 1))));
    let admin = testkit::admin_token();
    let key = format!("edge-rl.{}", uuid::Uuid::new_v4());
    let put = || {
        req(
            "PUT",
            &format!("/api/decisions/{key}"),
            Some(&admin),
            Some(json!({"kind": "shared", "jdm": jdm()})),
            None,
        )
    };

    // PUT #1: global(1) + mutation(1) → 200.
    let res = app.clone().oneshot(put()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // PUT #2: global(2) ok, mutation bucket empty → 429 + Retry-After.
    let res = app.clone().oneshot(put()).await.unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(res.headers().contains_key("retry-after"));
    let body: Value =
        serde_json::from_slice(&to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["error"], json!("rate limited (mutation bucket)"));

    // GET: global(3) still within budget → 200.
    let res = app
        .clone()
        .oneshot(req("GET", "/api/health", None, None, None))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Next request: global bucket empty → 429.
    let res = app
        .oneshot(req("GET", "/api/health", None, None, None))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);

    sqlx::query("DELETE FROM decisions WHERE key = $1")
        .bind(&key)
        .execute(&pool)
        .await
        .unwrap();
}

/// The mutation tier covers the rate-index admin POSTs (MPE-152): preview
/// and import draw the tight bucket, while compute POSTs (evaluate) stay
/// on the global tier — exhausting mutations must not block evaluation.
#[tokio::test]
async fn rate_index_posts_use_mutation_bucket() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping edge test");
        return;
    };
    // global=4, mutations=1 on the shared "unknown" bucket.
    let app = make_app_with_limits(&pool, Some(Arc::new(EdgeLimits::new(4, 1))));
    let admin = testkit::admin_token();
    let preview = || {
        req(
            "POST",
            "/api/admin/rate-index/preview",
            Some(&admin),
            None,
            None,
        )
    };

    // Preview #1: global(1) + mutation(1) → reaches the handler, which
    // 400s because RATE_INDEX_URL is unconfigured — not a 429.
    let res = app.clone().oneshot(preview()).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Preview #2: mutation bucket empty → 429 mutation + Retry-After.
    let res = app.clone().oneshot(preview()).await.unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(res.headers().contains_key("retry-after"));
    let body: Value =
        serde_json::from_slice(&to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["error"], json!("rate limited (mutation bucket)"));

    // Import draws the same tight bucket → 429 (global still has budget).
    let s = status(
        &app,
        req(
            "POST",
            "/api/admin/rate-index/import",
            Some(&admin),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);

    // Evaluate stays global-only: global 3/4 used → the compute path is
    // still served (200) even with the mutation tier exhausted.
    let s = status(
        &app,
        req(
            "POST",
            "/api/evaluate/eligibility",
            Some(&admin),
            Some(json!({
                "loan": {
                    "loan": { "ltv": 80.0 },
                    "borrower": { "creditScore": 740 },
                    "scenario": { "lockTermDays": 30 }
                }
            })),
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
}

/// PATCH is a mutation too (MPE-128): cell-patch requests draw the tight
/// mutation bucket, not just the loose global one.
#[tokio::test]
async fn patch_uses_mutation_bucket() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping edge test");
        return;
    };
    // global=3, mutations=1 on the shared "unknown" bucket.
    let app = make_app_with_limits(&pool, Some(Arc::new(EdgeLimits::new(3, 1))));
    let admin = testkit::admin_token();
    let patch = || {
        req(
            "PATCH",
            "/api/matrices/edge-patch/cells",
            Some(&admin),
            Some(json!({ "cells": { "rA:cX": 0.5 } })),
            None,
        )
    };

    // PATCH #1: global(1) + mutation(1) → reaches the handler, which 404s
    // (no draft) — proof it consumed both tokens, not a 429.
    let s = status(&app, patch()).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // PATCH #2: mutation bucket empty → 429 mutation + Retry-After. Were
    // PATCH classified global-only, this would 404 again instead.
    let res = app.clone().oneshot(patch()).await.unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(res.headers().contains_key("retry-after"));
    let body: Value =
        serde_json::from_slice(&to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["error"], json!("rate limited (mutation bucket)"));

    // The global bucket still had budget — the 429 came from the mutation
    // tier specifically.
    let s = status(&app, req("GET", "/api/health", None, None, None)).await;
    assert_eq!(s, StatusCode::OK);
}

/// With XFF trusted, each forwarded IP gets its own bucket.
#[tokio::test]
async fn xff_partitions_buckets_when_trusted() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping edge test");
        return;
    };
    let app = make_app_with_limits(&pool, Some(Arc::new(EdgeLimits::new(1, 1).trust_xff(true))));
    let get = |ip: &str| req("GET", "/api/health", None, None, Some(ip));

    assert_eq!(status(&app, get("203.0.113.1")).await, StatusCode::OK);
    assert_eq!(
        status(&app, get("203.0.113.1")).await,
        StatusCode::TOO_MANY_REQUESTS,
        "same forwarded IP shares one bucket"
    );
    assert_eq!(
        status(&app, get("203.0.113.2")).await,
        StatusCode::OK,
        "a different forwarded IP has its own bucket"
    );
}
