//! Compile-cache integration tests (MPE-127): a publish recompiles only the
//! keys whose content changed, and asOf snapshots resolve distinct compiled
//! entries per content hash. Gated on DATABASE_URL; skips locally otherwise.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use zen_engine::loader::DecisionLoader;

use mpex::api::{self, AppState};
use mpex::auth::testkit;
use mpex::rules::loader::{Env, Revision};
use mpex::rules::snapshot::SnapshotStore;

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

/// Publishing (here: drafting) one changed artifact recompiles exactly that
/// key — the fingerprint invalidates the snapshot, but every unchanged
/// content hash still hits the shared compile cache.
#[tokio::test]
async fn publish_recompiles_only_changed_keys() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping compile-cache test");
        return;
    };
    let (app, snapshots) = make_app(&pool);
    let admin = testkit::admin_token();
    let key = "cc-test.expression";

    // Seed the catalog + warm the cache: first build compiles everything.
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/expressions/{key}"),
        Some(&admin),
        Some(json!({ "kind": "standard", "expression": "1 + 1" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    snapshots
        .snapshot_for(Revision::Draft, Env::Dev)
        .await
        .expect("first snapshot builds");
    let warm = snapshots.compile_misses();
    assert!(warm > 0, "first build compiles the catalog");

    // Same fingerprint → cached snapshot, zero compiles.
    snapshots
        .snapshot_for(Revision::Draft, Env::Dev)
        .await
        .unwrap();
    assert_eq!(snapshots.compile_misses(), warm, "no rebuild, no compiles");

    // Change one artifact's content — draft fingerprint bumps, snapshot
    // rebuilds, but only this key's new hash misses.
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/expressions/{key}"),
        Some(&admin),
        Some(json!({ "kind": "standard", "expression": "2 + 2" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    snapshots
        .snapshot_for(Revision::Draft, Env::Dev)
        .await
        .unwrap();
    assert_eq!(
        snapshots.compile_misses(),
        warm + 1,
        "one changed key recompiles; the rest hit (misses={} → {})",
        warm,
        snapshots.compile_misses()
    );

    sqlx::query("DELETE FROM expressions WHERE key = $1 AND status = 'draft'")
        .bind(key)
        .execute(&pool)
        .await
        .unwrap();
}

/// Two published versions of one key produce distinct cache entries; an
/// asOf between them resolves the first version — never a stale hit.
#[tokio::test]
async fn asof_resolves_versioned_content() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping compile-cache test");
        return;
    };
    let (app, snapshots) = make_app(&pool);
    let admin = testkit::admin_token();
    let key = "cc-asof.expression";

    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/expressions/{key}"),
        Some(&admin),
        Some(json!({ "kind": "standard", "expression": "10" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(
        &app,
        "POST",
        "/api/publications",
        Some(&admin),
        Some(json!({ "label": format!("cc-v1-{}", uuid::Uuid::new_v4()) })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let mid = chrono::Utc::now();

    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/expressions/{key}"),
        Some(&admin),
        Some(json!({ "kind": "standard", "expression": "20" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(
        &app,
        "POST",
        "/api/publications",
        Some(&admin),
        Some(json!({ "label": format!("cc-v2-{}", uuid::Uuid::new_v4()) })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // asOf before v2's publish resolves v1's compiled entry; latest
    // resolves v2 — distinct (key, hash) entries, no stale hit.
    let asof = snapshots
        .snapshot_for(Revision::AsOf(mid), Env::Dev)
        .await
        .unwrap();
    let v1 = asof.loader.load(key).await.expect("asOf resolves the key");
    let latest = snapshots
        .snapshot_for(Revision::LatestPublished, Env::Dev)
        .await
        .unwrap();
    let v2 = latest
        .loader
        .load(key)
        .await
        .expect("latest resolves the key");
    assert_ne!(
        serde_json::to_value(&*v1).unwrap(),
        serde_json::to_value(&*v2).unwrap(),
        "asOf and latest resolve different published versions"
    );

    sqlx::query("DELETE FROM expressions WHERE key = $1 AND status = 'draft'")
        .bind(key)
        .execute(&pool)
        .await
        .unwrap();
}
