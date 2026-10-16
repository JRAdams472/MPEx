use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use zen_engine::{DecisionEngine, EvaluationSerializedOptions, EvaluationTraceKind};

use crate::error::AppError;
use crate::rules::loader::Revision;

/// How evaluate endpoints select the decision revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalRevision {
    Latest,
    Draft,
    AsOf(chrono::DateTime<chrono::Utc>),
}

impl EvalRevision {
    /// Parses the request `asOf` field: null/omitted -> latest published
    /// (with draft fallback), "draft" -> draft row, RFC3339 -> historic.
    pub fn parse(as_of: Option<&str>) -> Result<Self, AppError> {
        match as_of {
            None => Ok(Self::Latest),
            Some("draft") => Ok(Self::Draft),
            Some(ts) => chrono::DateTime::parse_from_rfc3339(ts)
                .map(|t| Self::AsOf(t.with_timezone(&chrono::Utc)))
                .map_err(|_| {
                    AppError::BadRequest(format!("asOf must be 'draft' or RFC3339, got {ts:?}"))
                }),
        }
    }

    pub fn to_loader_revision(self) -> Revision {
        match self {
            Self::Latest => Revision::LatestPublished,
            Self::Draft => Revision::Draft,
            Self::AsOf(ts) => Revision::AsOf(ts),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductRef {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EligibilityOutcome {
    pub product: ProductRef,
    pub eligible: bool,
    pub reasons: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EligibilityReport {
    pub revision: String,
    pub eligible: Vec<EligibilityOutcome>,
    pub ineligible: Vec<EligibilityOutcome>,
    pub elapsed_ms: f64,
}

/// Runs every active product's `eligibility.<code>` decision against the loan
/// in parallel and buckets the products into eligible / ineligible.
///
/// Decision output contract: `{ "eligible": bool, "reasons": [...] }` —
/// `reasons` lists why the product was rejected (typically collected via a
/// `reasons[]` output column on a fail-condition decision table).
pub async fn evaluate_eligibility(
    pool: &PgPool,
    engine: DecisionEngine,
    loan: &Value,
    revision: EvalRevision,
    trace: bool,
) -> Result<EligibilityReport, AppError> {
    let started = std::time::Instant::now();
    let products: Vec<(String, String)> =
        sqlx::query_as("SELECT code, name FROM products WHERE active ORDER BY code")
            .fetch_all(pool)
            .await?;

    let opts = EvaluationSerializedOptions {
        trace: if trace {
            EvaluationTraceKind::Default
        } else {
            EvaluationTraceKind::None
        },
        ..Default::default()
    };

    // zen Variable is Rc-based (!Send), so each evaluation runs on a
    // spawn_blocking thread with its own current-thread runtime — products
    // still evaluate in parallel.
    let handles: Vec<_> = products
        .iter()
        .map(|(code, name)| {
            let engine = engine.clone();
            let loan = loan.clone();
            let key = format!("eligibility.{code}");
            let product = ProductRef {
                code: code.clone(),
                name: name.clone(),
            };
            tokio::task::spawn_blocking(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("eval runtime");
                let outcome = rt.block_on(engine.evaluate_serialized(&key, loan.into(), opts));
                (product, outcome)
            })
        })
        .collect();

    let evals = futures::future::join_all(handles).await;

    let mut eligible = Vec::new();
    let mut ineligible = Vec::new();
    for join in evals {
        let (product, outcome) = match join {
            Ok(pair) => pair,
            Err(e) => {
                ineligible.push(EligibilityOutcome {
                    product: ProductRef {
                        code: "unknown".to_string(),
                        name: String::new(),
                    },
                    eligible: false,
                    reasons: vec![Value::String(format!("eval task failed: {e}"))],
                    result: None,
                    performance: None,
                    trace: None,
                });
                continue;
            }
        };
        match outcome {
            Ok(value) => {
                let result = value.get("result").cloned();
                let is_eligible = result
                    .as_ref()
                    .and_then(|r| r.get("eligible"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let reasons = result
                    .as_ref()
                    .and_then(|r| r.get("reasons"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let row = EligibilityOutcome {
                    product,
                    eligible: is_eligible,
                    reasons,
                    result,
                    performance: value
                        .get("performance")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    trace: value.get("trace").cloned(),
                };
                if is_eligible {
                    eligible.push(row);
                } else {
                    ineligible.push(row);
                }
            }
            Err(err_value) => {
                ineligible.push(EligibilityOutcome {
                    product,
                    eligible: false,
                    reasons: vec![json_reason(&err_value)],
                    result: None,
                    performance: None,
                    trace: None,
                });
            }
        }
    }

    Ok(EligibilityReport {
        revision: match revision {
            EvalRevision::Latest => "latest".to_string(),
            EvalRevision::Draft => "draft".to_string(),
            EvalRevision::AsOf(ts) => ts.to_rfc3339(),
        },
        eligible,
        ineligible,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
    })
}

fn json_reason(err: &Value) -> Value {
    match err {
        Value::String(s) => Value::String(format!("decision error: {s}")),
        other => serde_json::json!({ "decisionError": other }),
    }
}
