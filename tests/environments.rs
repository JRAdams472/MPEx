//! Environment-lane tests (MPE-153): promotion adjacency and only-changed
//! stamping, env-scoped eval resolution, drafts invisible outside dev, and
//! the `role_grants.envs` permitted-lane gate.
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
use mpex::rules::snapshot::SnapshotStore;

fn make_app(pool: &PgPool) -> Router {
    api::router(AppState {
        pool: pool.clone(),
        snapshots: Arc::new(SnapshotStore::new(pool.clone())),
        auth: Some(testkit::test_context(pool.clone())),
        rate_limits: None,
        rate_index: None,
    })
}

fn req(method: &str, uri: &str, token: &str, body: Option<Value>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(match body {
            Some(b) => Body::from(serde_json::to_vec(&b).unwrap()),
            None => Body::empty(),
        })
        .unwrap()
}

async fn call(app: &Router, r: Request<Body>) -> (StatusCode, Value) {
    let res = app.clone().oneshot(r).await.unwrap();
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

fn loan() -> Value {
    json!({
        "loan": { "ltv": 80.0 },
        "borrower": { "creditScore": 740 },
        "scenario": { "lockTermDays": 30 }
    })
}

/// Find one product's eligibility outcome inside an eval report.
fn outcome<'a>(report: &'a Value, code: &str) -> Option<&'a Value> {
    report["eligible"]
        .as_array()
        .into_iter()
        .chain(report["ineligible"].as_array())
        .flatten()
        .find(|e| e["product"]["code"] == json!(code))
}

async fn eval_env(app: &Router, token: &str, env: Option<&str>) -> (StatusCode, Value) {
    let mut body = json!({ "loan": loan() });
    if let Some(e) = env {
        body["env"] = json!(e);
    }
    call(
        app,
        req("POST", "/api/evaluate/eligibility", token, Some(body)),
    )
    .await
}

/// Promote adjacency: only dev→qa, qa→uat, uat→prod are legal.
#[tokio::test]
async fn promotion_requires_adjacent_lanes() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping env test");
        return;
    };
    let app = make_app(&pool);
    let admin = testkit::admin_token();

    for (from, to) in [
        ("dev", "uat"),  // skips qa
        ("dev", "prod"), // skips two
        ("uat", "dev"),  // backwards
        ("qa", "dev"),
        ("prod", "dev"),
        ("dev", "dev"),
    ] {
        let (status, _) = call(
            &app,
            req(
                "POST",
                "/api/admin/promotions",
                &admin,
                Some(json!({ "from": from, "to": to, "label": "x" })),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{from}→{to} must be rejected"
        );
    }
    // Bad env names are rejected before adjacency.
    let (status, _) = call(
        &app,
        req(
            "POST",
            "/api/admin/promotions",
            &admin,
            Some(json!({ "from": "dev", "to": "mars", "label": "x" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Full lane flow: draft → dev publish → promote → eval resolves per env;
/// second promote with no new changes → 409; drafts never leak to qa.
#[tokio::test]
async fn promote_stamps_changed_artifacts_into_next_lane() {
    const PRODUCT: &str = "ENVP";
    const DECISION: &str = "eligibility.ENVP";

    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping env test");
        return;
    };
    // Clean slate.
    sqlx::query("DELETE FROM decisions WHERE key = $1")
        .bind(DECISION)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM publications WHERE label LIKE 'env-test-%'")
        .execute(&pool)
        .await
        .unwrap();
    for table in ["decisions", "expressions", "matrices", "rate_sheets"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE publication_id IN
             (SELECT id FROM publications WHERE label LIKE 'env-test-%')"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO products (code, name, loan_type, amortization, term_months)
         VALUES ($1, 'Env Test', 'conventional', 'fixed', 360)
         ON CONFLICT (code) DO NOTHING",
    )
    .bind(PRODUCT)
    .execute(&pool)
    .await
    .unwrap();

    let app = make_app(&pool);
    let admin = testkit::admin_token();
    let viewer = testkit::viewer_token();

    // Draft the decision (dev lane), publish to dev.
    let (status, body) = call(
        &app,
        req(
            "PUT",
            &format!("/api/decisions/{DECISION}"),
            &admin,
            Some(json!({ "kind": "eligibility", "productCode": PRODUCT, "jdm": jdm() })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "draft PUT failed: {body}");
    let (status, body) = call(
        &app,
        req(
            "POST",
            "/api/publications",
            &admin,
            Some(json!({ "label": "env-test-v1" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "dev publish failed: {body}");
    assert_eq!(body["env"], json!("dev"), "publish stamps the dev lane");

    // Dev eval: ENVP eligible. Omitted env = dev too (back-compat).
    for env in [None, Some("dev")] {
        let (status, report) = eval_env(&app, &viewer, env).await;
        assert_eq!(status, StatusCode::OK, "env {env:?} eval failed");
        let o = outcome(&report, PRODUCT).expect("ENVP missing from report");
        assert_eq!(o["eligible"], json!(true), "env {env:?} should be eligible");
    }

    // qa eval: the decision doesn't exist there yet → ineligible.
    let (status, report) = eval_env(&app, &viewer, Some("qa")).await;
    assert_eq!(status, StatusCode::OK);
    let o = outcome(&report, PRODUCT).expect("ENVP missing");
    assert_eq!(
        o["eligible"],
        json!(false),
        "qa must not see dev artifacts before promotion"
    );

    // Draft invisibility: a second unpublished draft can't reach qa either.
    let (status, _) = call(
        &app,
        req(
            "PUT",
            "/api/decisions/eligibility.ENVP2",
            &admin,
            Some(json!({ "kind": "eligibility", "productCode": PRODUCT, "jdm": jdm() })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, report) = eval_env(&app, &viewer, Some("qa")).await;
    let o = outcome(&report, PRODUCT).unwrap();
    assert_eq!(o["eligible"], json!(false), "qa must not see drafts");

    // Promote dev→qa.
    let (status, body) = call(
        &app,
        req(
            "POST",
            "/api/admin/promotions",
            &admin,
            Some(json!({ "from": "dev", "to": "qa", "label": "env-test-promote1" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "promote failed: {body}");
    assert_eq!(body["env"], json!("qa"));
    assert!(body["decisions"]
        .as_array()
        .unwrap()
        .contains(&json!(DECISION)));

    // qa now resolves ENVP eligible.
    let (status, report) = eval_env(&app, &viewer, Some("qa")).await;
    assert_eq!(status, StatusCode::OK);
    let o = outcome(&report, PRODUCT).unwrap();
    assert_eq!(
        o["eligible"],
        json!(true),
        "qa should see promoted decision"
    );

    // Admin asOf list honors ?env=: the qa lane shows the promoted key.
    let (status, body) = call(
        &app,
        req(
            "GET",
            "/api/decisions?asOf=2999-01-01T00:00:00Z&env=qa",
            &admin,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let keys: Vec<&str> = body["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["key"].as_str())
        .collect();
    assert!(
        keys.contains(&DECISION),
        "qa asOf list must include the promoted key: {body}"
    );

    // Second promote with nothing new in dev → 409 (draft ENVP2 is still a
    // draft, and ENVP is identical in both lanes).
    let (status, _) = call(
        &app,
        req(
            "POST",
            "/api/admin/promotions",
            &admin,
            Some(json!({ "from": "dev", "to": "qa", "label": "env-test-promote2" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "empty promote must 409");

    // Publish ENVP2's draft → promote again → only the new key stamps.
    let (status, _) = call(
        &app,
        req(
            "POST",
            "/api/publications",
            &admin,
            Some(json!({ "label": "env-test-v2" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = call(
        &app,
        req(
            "POST",
            "/api/admin/promotions",
            &admin,
            Some(json!({ "from": "dev", "to": "qa", "label": "env-test-promote3" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let stamped = body["decisions"].as_array().unwrap();
    assert!(
        stamped.contains(&json!("eligibility.ENVP2")),
        "new decision must promote: {body}"
    );
    assert!(
        !stamped.contains(&json!(DECISION)),
        "unchanged decision must not restamp: {body}"
    );

    // Promotion wrote an attributed audit row.
    let row: (String,) = sqlx::query_as(
        "SELECT action FROM audit_events
         WHERE action = 'promotion.create' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.0, "promotion.create");

    // Cleanup.
    for key in [DECISION, "eligibility.ENVP2"] {
        sqlx::query("DELETE FROM decisions WHERE key = $1")
            .bind(key)
            .execute(&pool)
            .await
            .unwrap();
    }
    for table in ["decisions", "expressions", "matrices", "rate_sheets"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE publication_id IN
             (SELECT id FROM publications WHERE label LIKE 'env-test-%')"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("DELETE FROM publications WHERE label LIKE 'env-test-%'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM products WHERE code = $1")
        .bind(PRODUCT)
        .execute(&pool)
        .await
        .unwrap();
}

/// `role_grants.envs` restricts eval lanes: a principal limited to 'prod'
/// gets 403 asking for dev; NULL grants stay unrestricted.
#[tokio::test]
async fn permitted_envs_gate_eval_lanes() {
    const SUB: &str = "test-env-prod-only";

    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL unset — skipping env test");
        return;
    };
    sqlx::query(
        "INSERT INTO role_grants (subject_type, subject, role, granted_by, envs)
         VALUES ('sub', $1, 'viewer', 'testkit', '{prod}')
         ON CONFLICT DO NOTHING",
    )
    .bind(SUB)
    .execute(&pool)
    .await
    .unwrap();

    let app = make_app(&pool);
    let token = testkit::token(SUB);

    // 'prod' lane permitted, 'dev' forbidden — regardless of which selector
    // (the lane gate runs before snapshot resolution).
    let (status, _) = eval_env(&app, &token, Some("prod")).await;
    assert_eq!(status, StatusCode::OK, "permitted lane must eval");
    let (status, body) = eval_env(&app, &token, Some("dev")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    // Omitted env defaults to dev → also 403 for a prod-only principal.
    let (status, _) = eval_env(&app, &token, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The canonical viewer grant (envs NULL) is unrestricted.
    let (status, _) = eval_env(&app, &testkit::viewer_token(), Some("dev")).await;
    assert_eq!(status, StatusCode::OK);

    sqlx::query("DELETE FROM role_grants WHERE subject = $1")
        .bind(SUB)
        .execute(&pool)
        .await
        .unwrap();
}
