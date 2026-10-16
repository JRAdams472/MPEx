use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use zen_engine::DecisionEngine;

use crate::api::AppState;
use crate::domain::loan::LoanScenario;
use crate::error::AppError;
use crate::pricing::eligibility::{evaluate_eligibility, EvalRevision};

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
