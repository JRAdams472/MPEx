//! PII-inbound guard + log-redaction (MPE-97 phase 6): /api/evaluate/*
//! rejects personal-data fields and value shapes with path-only 400s, and
//! eval payloads never appear in tracing output — proven against a captured
//! JSON log sink on a global subscriber (eval threads are !Send workers, so
//! only the global dispatcher sees their spans).
//! Gated on DATABASE_URL (set in CI); skips locally otherwise.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

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

async fn call(app: &Router, uri: &str, token: &str, body: Value) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
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
    testkit::ensure_grants(&pool).await;
    Some(pool)
}

/// Denylist names, value shapes, and deep nesting all 400 — and the error
/// carries the dotted path, never the offending value.
#[tokio::test]
async fn evaluate_rejects_pii_with_path_only() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping pii test");
        return;
    };
    let app = make_app(&pool);
    let admin = testkit::admin_token();

    let cases: Vec<(Value, &str, &str)> = vec![
        // (loan payload, expected path, sentinel value that must not leak)
        (
            json!({"borrower":{"ssn":"123-45-6789"}}),
            "borrower.ssn",
            "123-45-6789",
        ),
        (
            json!({"borrower":{"firstName":"Jane"}}),
            "borrower.firstName",
            "Jane",
        ),
        (
            json!({"loan":{"amount":450000},"notes":"call acct 123456789 now"}),
            "notes",
            "123456789",
        ),
        (
            json!({"borrower":{"alt":[{"contact":{"homePhone":"x"}}]}}),
            "borrower.alt[0].contact.homePhone",
            "homePhone",
        ),
    ];
    for (loan, path, sentinel) in cases {
        let (status, body) = call(
            &app,
            "/api/evaluate/eligibility",
            &admin,
            json!({ "loan": loan }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} must be denied: {body}"
        );
        assert_eq!(body["error"], json!(format!("PII rejected: {path}")));
        assert!(
            !body.to_string().contains(sentinel) || sentinel == "homePhone",
            "value must never appear in the response: {body}"
        );
    }

    // Legit loan passes the guard (empty catalog → empty eligibility, not 400).
    let (status, _) = call(
        &app,
        "/api/evaluate/eligibility",
        &admin,
        json!({ "loan": {
            "loan": {"purpose": "purchase", "type": "conventional", "amount": 450000,
                     "termMonths": 360, "ltv": 80.0, "dti": 36.5},
            "property": {"type": "sfr", "occupancy": "primary", "state": "TX"},
            "borrower": {"creditScore": 740, "existingLiens": 0},
            "scenario": {"lockTermDays": 30}
        }}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

/// In-memory log sink: every tracing event lands in `buf` as JSON lines.
#[derive(Clone, Default)]
struct LogBuf(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuf {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuf {
    type Writer = LogBuf;
    fn make_writer(&'a self) -> LogBuf {
        self.clone()
    }
}

/// Sentinel PII values — one accepted-shaped extension field, one denied
/// field — must never appear in any log line or span.
#[tokio::test]
async fn eval_payloads_never_reach_logs() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping pii test");
        return;
    };
    let buf = LogBuf::default();
    let sub = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(buf.clone())
        .finish();
    // Global (not thread-local): eval runs on other threads.
    tracing::subscriber::set_global_default(sub).expect("global default unset");

    let app = make_app(&pool);
    let admin = testkit::admin_token();
    let alpha = "PII-SENTINEL-ALPHA-77";
    let beta = "PII-SENTINEL-BETA-88";

    // Sentinel in a legal extension field: passes the guard, flows through
    // snapshot resolution and the eval engine — the deepest path possible.
    let (status, _) = call(
        &app,
        "/api/evaluate/eligibility",
        &admin,
        json!({ "loan": {
            "loan": {"amount": 450000, "ltv": 80.0},
            "borrower": {"creditScore": 740},
            "notes": alpha
        }}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Denied field: the 400 path must not echo the value either.
    let (status, _) = call(
        &app,
        "/api/evaluate/eligibility",
        &admin,
        json!({ "loan": { "borrower": { "ssn": beta } } }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Let writers flush.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let logs = String::from_utf8_lossy(&buf.0.lock().unwrap()).to_string();
    assert!(!logs.is_empty(), "log sink captured nothing — test is void");
    assert!(
        !logs.contains(alpha),
        "extension-field sentinel leaked:\n{logs}"
    );
    assert!(
        !logs.contains(beta),
        "denied-field sentinel leaked:\n{logs}"
    );
}
