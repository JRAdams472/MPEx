//! Engine-level tests: the Zen spine end-to-end without Postgres
//! (MemoryLoader), plus a gated integration test against a live DB
//! (runs in CI where DATABASE_URL is set, skips locally otherwise).

use serde_json::json;
use zen_engine::loader::MemoryLoader;
use zen_engine::model::DecisionContent;
use zen_engine::{DecisionEngine, EvaluationSerializedOptions, EvaluationTraceKind, Variable};

fn eligibility_jdm(fail_expr: &str, reason: &str) -> serde_json::Value {
    json!({
        "nodes": [
            {"id": "in", "type": "inputNode", "name": "Request"},
            {"id": "tbl", "type": "decisionTableNode", "name": "Rules",
             "content": {
                "hitPolicy": "first",
                "inputs": [{"id": "i1", "name": "LTV", "field": "loan.ltv"}],
                "outputs": [{"id": "o1", "name": "reason", "field": "reasons[]"}],
                "rules": [{"_id": "r1", "i1": fail_expr, "o1": reason}]
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

#[tokio::test]
async fn eligibility_collects_fail_reasons() {
    let loader = MemoryLoader::default();
    let jdm = eligibility_jdm("> 80", "'ltv exceeds 80%'");
    loader.add(
        "eligibility.TEST",
        DecisionContent::from(
            serde_json::from_value::<zen_engine::model::GraphContent>(jdm).unwrap(),
        ),
    );
    let engine = DecisionEngine::default().with_loader(std::sync::Arc::new(loader));

    let ok: Variable = json!({ "loan": { "ltv": 75 } }).into();
    let res = engine
        .evaluate_serialized("eligibility.TEST", ok, Default::default())
        .await
        .unwrap();
    assert_eq!(res["result"]["eligible"], json!(true));
    assert_eq!(res["result"]["reasons"], json!([]));

    let bad: Variable = json!({ "loan": { "ltv": 90 } }).into();
    let res = engine
        .evaluate_serialized("eligibility.TEST", bad, Default::default())
        .await
        .unwrap();
    assert_eq!(res["result"]["eligible"], json!(false));
    assert_eq!(res["result"]["reasons"], json!(["ltv exceeds 80%"]));
}

#[tokio::test]
async fn trace_is_returned_when_requested() {
    let loader = MemoryLoader::default();
    let jdm = eligibility_jdm("> 80", "'ltv exceeds 80%'");
    loader.add(
        "eligibility.TEST",
        DecisionContent::from(
            serde_json::from_value::<zen_engine::model::GraphContent>(jdm).unwrap(),
        ),
    );
    let engine = DecisionEngine::default().with_loader(std::sync::Arc::new(loader));
    let ctx: Variable = json!({ "loan": { "ltv": 90 } }).into();
    let res = engine
        .evaluate_serialized(
            "eligibility.TEST",
            ctx,
            EvaluationSerializedOptions {
                trace: EvaluationTraceKind::Default,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(res.get("trace").is_some(), "trace should be present");
}

#[tokio::test]
async fn live_api_smoke() {
    let Ok(base) = std::env::var("MPEX_API_URL") else {
        eprintln!("MPEX_API_URL unset — skipping live smoke test");
        return;
    };
    let client = reqwest::Client::new();
    let loan = json!({
        "loan": {"purpose": "purchase", "type": "conventional", "amount": 450000,
                 "termMonths": 360, "amortization": "fixed", "ltv": 80.0, "dti": 36, "units": 1},
        "property": {"type": "sfr", "occupancy": "primary", "state": "TX"},
        "borrower": {"fico": 740},
        "scenario": {"lockTermDays": 30}
    });
    let res: serde_json::Value = client
        .post(format!("{base}/api/evaluate/eligibility"))
        .json(&json!({ "loan": loan }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let codes: Vec<_> = res["eligible"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["product"]["code"].as_str())
        .collect();
    assert!(
        codes.contains(&"CONV30"),
        "CONV30 should be eligible: {res}"
    );
}
