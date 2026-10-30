//! MND rate-index import integration tests (MPE-137): gating, preview,
//! import → draft upserts → rate-sheets-scoped publish → provenance +
//! audit, atomic failure on a changed page, and rate-config validation.
//! Gated on DATABASE_URL (set in CI); skips locally otherwise. The index
//! page is stubbed by a local axum server — tests never hit the network.

use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

use mpex::api::{self, AppState};
use mpex::auth::testkit;
use mpex::rate_index::IndexSettings;
use mpex::rules::snapshot::SnapshotStore;

const FIXTURE: &str = include_str!("fixtures/mnd_rates.html");

fn make_app(pool: &PgPool, index_url: &str) -> Router {
    api::router(AppState {
        pool: pool.clone(),
        snapshots: Arc::new(SnapshotStore::new(pool.clone())),
        auth: Some(testkit::test_context(pool.clone())),
        rate_limits: None,
        rate_index: Some(IndexSettings {
            url: index_url.into(),
            timeout: Duration::from_secs(10),
        }),
    })
}

/// Serves `html` for every path on an ephemeral port; returns the base URL.
async fn stub_index(html: &'static str) -> String {
    let app = Router::new().fallback(get(move || async move { Html(html) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/mortgage-rates/mnd")
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

/// Deterministic catalog for the resolve path: CONV30 base, DERV30 derived
/// (-0.25 off CONV30), MANUALX with no rate source. Idempotent upserts keep
/// the test stable on a shared dev database.
async fn seed_products(pool: &PgPool) {
    for (code, name, label, base, adj) in [
        ("CONV30", "Conventional 30", Some("30 Yr. Fixed"), None, 0.0),
        ("DERV30", "Derived 30", None, Some("CONV30"), -0.25),
        ("MANUALX", "Manual product", None, None, 0.0),
    ] {
        sqlx::query(
            "INSERT INTO products (code, name, loan_type, amortization, term_months,
                                   description, mnd_label, base_product_code, rate_adjustment)
             VALUES ($1, $2, 'conventional', 'fixed', 360, '', $3, $4, $5)
             ON CONFLICT (code) DO UPDATE
               SET mnd_label = EXCLUDED.mnd_label,
                   base_product_code = EXCLUDED.base_product_code,
                   rate_adjustment = EXCLUDED.rate_adjustment",
        )
        .bind(code)
        .bind(name)
        .bind(label)
        .bind(base)
        .bind(adj)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// Published sheet rate for a product, if any.
async fn published_rate(pool: &PgPool, code: &str) -> Option<f64> {
    sqlx::query_scalar::<_, Value>(
        "SELECT spec FROM rate_sheets
         WHERE key = $1 AND status = 'published'
         ORDER BY published_at DESC LIMIT 1",
    )
    .bind(code)
    .fetch_optional(pool)
    .await
    .unwrap()
    .and_then(|spec| spec["rates"].as_array()?.first()?.as_f64())
}

#[tokio::test]
async fn rate_index_gating() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping rate_index test");
        return;
    };
    let url = stub_index(FIXTURE).await;
    let app = make_app(&pool, &url);

    // GET is viewer+ like the other admin reads.
    let viewer = testkit::token(testkit::SUB_VIEWER);
    let (status, body) = call(&app, "GET", "/api/rate-index", Some(&viewer), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["indexLabels"].as_array().unwrap().len(), 6);

    for (method, uri) in [
        ("POST", "/api/admin/rate-index/preview"),
        ("POST", "/api/admin/rate-index/import"),
    ] {
        let (status, _) = call(&app, method, uri, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri} no token");
        let pricer = testkit::token(testkit::SUB_PRICER);
        let (status, _) = call(&app, method, uri, Some(&pricer), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri} pricer");
    }
}

#[tokio::test]
async fn preview_resolves_without_writing() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping rate_index test");
        return;
    };
    seed_products(&pool).await;
    let url = stub_index(FIXTURE).await;
    let app = make_app(&pool, &url);
    let admin = testkit::token(testkit::SUB_ADMIN);

    let (status, body) = call(
        &app,
        "POST",
        "/api/admin/rate-index/preview",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let labels: Vec<&str> = body["indexRates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels.len(), 6);
    assert!(labels.contains(&"7/6 SOFR ARM"));

    let sheet = |code: &str| {
        body["sheets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["productCode"] == code)
            .cloned()
    };
    assert_eq!(sheet("CONV30").unwrap()["rate"], json!(7.29));
    assert_eq!(
        sheet("CONV30").unwrap()["source"],
        json!("index:30 Yr. Fixed")
    );
    // Derived: 7.29 - 0.25.
    assert_eq!(sheet("DERV30").unwrap()["rate"], json!(7.04));
    assert_eq!(sheet("DERV30").unwrap()["source"], json!("base:CONV30"));
    // VA30 arrives from the migration's index-label mapping.
    assert_eq!(sheet("VA30").unwrap()["rate"], json!(6.92));
    // MANUALX has no rate source → skipped. `unmapped` lists index labels
    // with no product — env-dependent (a migration-only CI DB maps just
    // CONV30+VA30), so assert the invariant: mapped labels never appear
    // and every entry is a real index label.
    assert!(body["skipped"]
        .as_array()
        .unwrap()
        .contains(&json!("MANUALX")));
    let unmapped: Vec<String> = serde_json::from_value(body["unmapped"].clone()).unwrap();
    assert!(!unmapped.iter().any(|l| l == "30 Yr. Fixed"));
    assert!(!unmapped.iter().any(|l| l == "30 Yr. VA"));
    assert!(unmapped
        .iter()
        .all(|l| mpex::rate_index::parser::INDEX_LABELS.contains(&l.as_str())));

    // Preview wrote nothing — scoped to this stub's URL so parallel tests
    // importing through their own stubs don't race the assertion.
    let imports: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_index_imports WHERE source_url = $1")
            .bind(&url)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(imports, 0);
}

#[tokio::test]
async fn import_writes_sheets_publishes_and_audits() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping rate_index test");
        return;
    };
    seed_products(&pool).await;
    let url = stub_index(FIXTURE).await;
    let app = make_app(&pool, &url);
    let admin = testkit::token(testkit::SUB_ADMIN);

    // An expression draft that must NOT be swept into the rate-sheet
    // publish. An expression (not a decision) so it stays eval-safe even if
    // a parallel test's catalog-wide eval touches it.
    sqlx::query(
        "INSERT INTO expressions (key, kind, expression, status)
         VALUES ('test.ri-scope', 'standard', '1', 'draft')
         ON CONFLICT DO NOTHING",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = call(
        &app,
        "POST",
        "/api/admin/rate-index/import",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // Single-rate-at-par published sheets.
    assert_eq!(published_rate(&pool, "CONV30").await, Some(7.29));
    assert_eq!(published_rate(&pool, "DERV30").await, Some(7.04));
    assert_eq!(published_rate(&pool, "VA30").await, Some(6.92));
    // Manual product untouched.
    assert_eq!(published_rate(&pool, "MANUALX").await, None);

    let spec: Value = sqlx::query_scalar(
        "SELECT spec FROM rate_sheets WHERE key = 'CONV30' AND status = 'published'
         ORDER BY published_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(spec["lockTerms"], json!([30]));
    assert_eq!(spec["prices"]["7.29:30"], json!(100.0));

    // Scoped publish: the stray expression draft is still a draft (publish
    // copies, never deletes — the draft row survives unrelated publishes).
    let expr_drafts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM expressions WHERE key = 'test.ri-scope' AND status = 'draft'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(expr_drafts, 1);

    // On a fresh test DB the import stamps a publication; on a dev DB where
    // these exact sheets already published, change-detection yields none —
    // either way `publication` and the import row's publication_id agree.
    // When a publication was stamped it must be sheet-scoped (0 expressions).
    if let Some(pub_id) = body["publication"]["id"].as_str() {
        let pub_expressions: i64 =
            sqlx::query_scalar("SELECT count(*) FROM expressions WHERE publication_id = $1")
                .bind(pub_id.parse::<uuid::Uuid>().unwrap())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(pub_expressions, 0);
    }

    // Provenance + audit.
    let (imported_by, pub_id): (String, Option<uuid::Uuid>) =
        sqlx::query_as("SELECT imported_by, publication_id FROM rate_index_imports WHERE id = $1")
            .bind(
                body["importId"]
                    .as_str()
                    .unwrap()
                    .parse::<uuid::Uuid>()
                    .unwrap(),
            )
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(imported_by, testkit::SUB_ADMIN);
    match body["publication"]["id"].as_str() {
        Some(id) => assert_eq!(pub_id.map(|p| p.to_string()).as_deref(), Some(id)),
        None => assert!(pub_id.is_none()),
    }

    let (status, body) = call(
        &app,
        "GET",
        "/api/audit?action=rate_index.import&limit=5",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["actorSub"] == testkit::SUB_ADMIN
                && e["artifactTable"] == "rate_index_imports")
    );

    // Re-import with the same index: drafts identical → no new publication,
    // but the import is still recorded with a NULL publication_id.
    let (status, body) = call(
        &app,
        "POST",
        "/api/admin/rate-index/import",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert!(body["publication"].is_null(), "{body}");
    let reimport_pub: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT publication_id FROM rate_index_imports WHERE id = $1")
            .bind(
                body["importId"]
                    .as_str()
                    .unwrap()
                    .parse::<uuid::Uuid>()
                    .unwrap(),
            )
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(reimport_pub.is_none());
}

#[tokio::test]
async fn import_is_atomic_on_bad_index() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping rate_index test");
        return;
    };
    // Jumbo row renamed → parser fails before the transaction opens, so
    // nothing writes. No products needed — the parse error comes first.
    let broken: &'static str = Box::leak(
        FIXTURE
            .replace("30 Yr. Jumbo", "30 Yr. Jmbo")
            .into_boxed_str(),
    );
    let url = stub_index(broken).await;
    let app = make_app(&pool, &url);
    let admin = testkit::token(testkit::SUB_ADMIN);

    let (status, body) = call(
        &app,
        "POST",
        "/api/admin/rate-index/import",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");

    // Scoped by this stub's URL — parallel tests' imports don't race.
    let imports: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_index_imports WHERE source_url = $1")
            .bind(&url)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(imports, 0);
}

#[tokio::test]
async fn rate_config_validation_and_audit() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping rate_index test");
        return;
    };
    seed_products(&pool).await;
    let url = stub_index(FIXTURE).await;
    let app = make_app(&pool, &url);
    let admin = testkit::token(testkit::SUB_ADMIN);
    let path = "/api/admin/products/DERV30/rate-config";

    for bad in [
        // Unknown index label.
        json!({"mndLabel": "10 Yr. Fixed"}),
        // Both source kinds at once.
        json!({"mndLabel": "30 Yr. Fixed", "baseProductCode": "CONV30"}),
        // Self-derivation.
        json!({"baseProductCode": "DERV30"}),
        // Base that isn't an index product.
        json!({"baseProductCode": "MANUALX"}),
        // Adjustment without a base.
        json!({"rateAdjustment": 0.5}),
        // Absurd adjustment.
        json!({"baseProductCode": "CONV30", "rateAdjustment": 9.0}),
    ] {
        let (status, body) = call(&app, "PUT", path, Some(&admin), Some(bad.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad} -> {body}");
    }

    let (status, _) = call(
        &app,
        "PUT",
        "/api/admin/products/NOPE/rate-config",
        Some(&admin),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Valid: DERV30 derived from CONV30 at -0.25, audited.
    let (status, body) = call(
        &app,
        "PUT",
        path,
        Some(&admin),
        Some(json!({"baseProductCode": "CONV30", "rateAdjustment": -0.25})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rateAdjustment"], json!(-0.25));

    let (status, body) = call(
        &app,
        "GET",
        "/api/audit?action=product.rate_config.put&limit=5",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["artifactKey"] == "DERV30" && e["actorSub"] == testkit::SUB_ADMIN));
}
