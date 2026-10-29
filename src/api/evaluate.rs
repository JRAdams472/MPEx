use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use zen_engine::DecisionEngine;

use crate::api::AppState;
use crate::domain::loan::LoanScenario;
use crate::error::AppError;
use crate::pricing::eligibility::{evaluate_eligibility, EvalRevision};
use crate::pricing::pipeline::{evaluate_pricing, PricingParams};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateRequest {
    pub loan: Value,
    /// Revision selector: omitted = latest published (draft fallback),
    /// "draft", or an RFC3339 timestamp for historic evaluation.
    pub as_of: Option<String>,
    #[serde(default)]
    pub trace: bool,
}

pub async fn eligibility(
    State(state): State<AppState>,
    Json(req): Json<EvaluateRequest>,
) -> Result<Json<Value>, AppError> {
    // PII guard before the model parse: reject personal-data fields/values
    // outright — the loan model carries loan attributes only.
    crate::security::pii_guard::check_loan(&req.loan)?;
    LoanScenario::parse(&req.loan)
        .map_err(|e| AppError::BadRequest(format!("invalid loan model: {e}")))?;
    let revision = EvalRevision::parse(req.as_of.as_deref())?;

    // Resolve the full decision snapshot on the async runtime; evaluation
    // threads then run pure (no DB I/O inside the engine).
    let loader = state
        .snapshots
        .loader_for(revision.to_loader_revision())
        .await
        .map_err(AppError::Internal)?;
    let engine = DecisionEngine::default().with_loader(loader);

    let report = evaluate_eligibility(&state.pool, engine, &req.loan, revision, req.trace).await?;
    Ok(Json(serde_json::to_value(report).unwrap_or_default()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingRequest {
    pub loan: Value,
    /// Product codes to price (eligibility is a prior, separate call).
    /// Unlisted products are never priced.
    #[serde(default)]
    pub products: Vec<String>,
    /// Pin pricing to one rate; omitted = a line per offered rate.
    pub rate: Option<f64>,
    /// Lock term in days; falls back to loan.scenario.lockTermDays, then the
    /// sheet's shortest term.
    pub lock_term_days: Option<u32>,
    pub as_of: Option<String>,
    #[serde(default)]
    pub trace: bool,
}

/// Full pricing pass: rate-sheet base price, then applicable LLPA matrices.
/// Responses memoize on (snapshot fingerprint, loan, products, rate, term,
/// trace) — evaluation is a pure function of those, and a publish bumps the
/// fingerprint so stale entries never serve.
pub async fn pricing(
    State(state): State<AppState>,
    Json(req): Json<PricingRequest>,
) -> Result<Json<Value>, AppError> {
    crate::security::pii_guard::check_loan(&req.loan)?;
    LoanScenario::parse(&req.loan)
        .map_err(|e| AppError::BadRequest(format!("invalid loan model: {e}")))?;
    let revision = EvalRevision::parse(req.as_of.as_deref())?;
    let snapshot = state
        .snapshots
        .snapshot_for(revision.to_loader_revision())
        .await
        .map_err(AppError::Internal)?;

    let mut products = req.products.clone();
    products.sort();
    products.dedup();
    let memo_key = format!(
        "{}|{}|{}|{:?}|{:?}|{}",
        snapshot.fingerprint,
        req.loan,
        products.join(","),
        req.rate,
        req.lock_term_days,
        req.trace
    );
    if let Some(hit) = state.snapshots.memo_get(&memo_key) {
        return Ok(Json((*hit).clone()));
    }

    let engine = DecisionEngine::default().with_loader(snapshot.loader.clone());
    let params = PricingParams {
        products: &products,
        rate: req.rate,
        lock_term_days: req.lock_term_days,
        trace: req.trace,
    };
    let report =
        evaluate_pricing(&state.pool, &snapshot, engine, &req.loan, &params, revision).await?;
    let body = serde_json::to_value(report).unwrap_or_default();
    state.snapshots.memo_put(memo_key, body.clone());
    Ok(Json(body))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidateExpressionRequest {
    pub expression: String,
    /// "standard" (default) or "unary" (decision-table cell expressions like `> 80`)
    pub kind: Option<String>,
}

pub async fn validate_expression(Json(req): Json<ValidateExpressionRequest>) -> Json<Value> {
    let result = match req.kind.as_deref() {
        Some("unary") => zen_expression::compile_unary_expression(&req.expression)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        _ => zen_expression::compile_expression(&req.expression)
            .map(|_| ())
            .map_err(|e| e.to_string()),
    };
    match result {
        Ok(()) => Json(json!({ "ok": true })),
        Err(error) => Json(json!({ "ok": false, "error": error })),
    }
}
