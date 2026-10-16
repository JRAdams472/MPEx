use serde::Serialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use zen_engine::{DecisionEngine, EvaluationSerializedOptions, EvaluationTraceKind};

use crate::error::AppError;
use crate::pricing::eligibility::{EvalRevision, ProductRef};
use crate::rules::rate_sheet::RateSheetSpec;
use crate::rules::snapshot::Snapshot;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjustmentLine {
    /// Matrix key that produced this waterfall entry.
    pub matrix: String,
    /// Aggregated matrix output (`op` applied to `adjustments`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Raw collected cell values from the compiled table.
    pub adjustments: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<Value>,
    /// Set when the matrix failed to evaluate; contributes 0 to the price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingLine {
    pub rate: f64,
    pub lock_term_days: u32,
    pub base_price: f64,
    pub adjustment_total: f64,
    /// basePrice + adjustmentTotal.
    pub price: f64,
    /// LLPA waterfall — one entry per applicable matrix.
    pub adjustments: Vec<AdjustmentLine>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricedProduct {
    pub product: ProductRef,
    pub rate_sheet: String,
    pub lines: Vec<PricingLine>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnpricedProduct {
    pub product: ProductRef,
    pub reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingReport {
    pub revision: String,
    pub priced: Vec<PricedProduct>,
    pub unpriced: Vec<UnpricedProduct>,
    pub elapsed_ms: f64,
}

/// Request-level pricing inputs — which products, at what rate/term.
pub struct PricingParams<'a> {
    /// Product codes to price; unlisted products are never priced.
    pub products: &'a [String],
    /// Pin pricing to one rate; None = a line per offered rate.
    pub rate: Option<f64>,
    /// Lock term in days; falls back to loan.scenario.lockTermDays, then the
    /// sheet's shortest term.
    pub lock_term_days: Option<u32>,
    pub trace: bool,
}

/// Prices each listed product in parallel: base price from the product's
/// rate sheet, then every applicable LLPA matrix (product-specific or
/// shared) evaluated against the loan plus `loan.rate` /
/// `loan.scenario.lockTermDays` for the line being priced.
///
/// Evaluation runs on spawn_blocking current-thread runtimes (zen `Variable`
/// is `Rc`-based/`!Send`) — no DB I/O inside.
#[tracing::instrument(
    name = "eval.pricing",
    skip(pool, snapshot, engine, loan, params),
    fields(
        revision = ?revision,
        products = params.products.len(),
        priced = tracing::field::Empty,
        unpriced = tracing::field::Empty,
        elapsed_ms = tracing::field::Empty
    )
)]
pub async fn evaluate_pricing(
    pool: &PgPool,
    snapshot: &Snapshot,
    engine: DecisionEngine,
    loan: &Value,
    params: &PricingParams<'_>,
    revision: EvalRevision,
) -> Result<PricingReport, AppError> {
    let started = std::time::Instant::now();

    let known: Vec<(String, String)> =
        sqlx::query_as("SELECT code, name FROM products WHERE code = ANY($1) ORDER BY code")
            .bind(params.products)
            .fetch_all(pool)
            .await?;
    let mut unpriced: Vec<UnpricedProduct> = params
        .products
        .iter()
        .filter(|c| !known.iter().any(|(k, _)| k == *c))
        .map(|c| UnpricedProduct {
            product: ProductRef {
                code: c.clone(),
                name: String::new(),
            },
            reason: "unknown product".into(),
        })
        .collect();

    // Resolve each product's inputs on the async runtime, then price on
    // blocking threads — one task per product.
    let loan_term = loan
        .get("scenario")
        .and_then(|s| s.get("lockTermDays"))
        .and_then(Value::as_u64)
        .map(|v| v as u32);

    struct Job {
        product: ProductRef,
        sheet_key: String,
        sheet: RateSheetSpec,
        matrix_keys: Vec<String>,
        term: u32,
        rates: Vec<f64>,
    }

    let mut jobs = Vec::new();
    for (code, name) in known {
        let product = ProductRef {
            code: code.clone(),
            name,
        };
        let Some(spec_json) = snapshot.rate_sheets.get(&code) else {
            unpriced.push(UnpricedProduct {
                product,
                reason: "no rate sheet".into(),
            });
            continue;
        };
        let sheet = match RateSheetSpec::parse(spec_json) {
            Ok(s) => s,
            Err(e) => {
                unpriced.push(UnpricedProduct {
                    product,
                    reason: format!("invalid rate sheet: {e}"),
                });
                continue;
            }
        };
        let term = params
            .lock_term_days
            .or(loan_term)
            .or_else(|| sheet.lock_terms.iter().min().copied());
        let Some(term) = term else {
            unpriced.push(UnpricedProduct {
                product,
                reason: "no lock term available".into(),
            });
            continue;
        };
        if !sheet.lock_terms.contains(&term) {
            unpriced.push(UnpricedProduct {
                product,
                reason: format!("lock term {term} not offered"),
            });
            continue;
        }
        let rates: Vec<f64> = match params.rate {
            Some(r) => {
                if !sheet.rates.contains(&r) {
                    unpriced.push(UnpricedProduct {
                        product,
                        reason: format!("rate {r} not on sheet"),
                    });
                    continue;
                }
                vec![r]
            }
            // Per-rate lines: every listed rate with a price at this term.
            None => sheet
                .rates
                .iter()
                .copied()
                .filter(|r| sheet.price(*r, term).is_some())
                .collect(),
        };
        if rates.is_empty() {
            unpriced.push(UnpricedProduct {
                product,
                reason: format!("no prices at lock term {term}"),
            });
            continue;
        }
        let matrix_keys = snapshot
            .matrices
            .iter()
            .filter(|m| {
                m.kind == "llpa"
                    && (m.product_code.is_none()
                        || m.product_code.as_deref() == Some(code.as_str()))
            })
            .map(|m| m.key.clone())
            .collect();
        jobs.push(Job {
            product,
            sheet_key: code,
            sheet,
            matrix_keys,
            term,
            rates,
        });
    }

    let opts = EvaluationSerializedOptions {
        trace: if params.trace {
            EvaluationTraceKind::Default
        } else {
            EvaluationTraceKind::None
        },
        ..Default::default()
    };

    let handles: Vec<_> = jobs
        .into_iter()
        .map(|job| {
            let engine = engine.clone();
            let loan = loan.clone();
            // Span is Send+Sync — the child span is created and entered inside
            // the blocking task so the eval stays under the request trace.
            let parent = tracing::Span::current();
            tokio::task::spawn_blocking(move || {
                let span = tracing::info_span!(
                    parent: &parent,
                    "eval.product",
                    product = %job.product.code,
                    rates = job.rates.len(),
                    matrices = job.matrix_keys.len()
                );
                let _g = span.enter();
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("eval runtime");
                let mut lines = Vec::new();
                for rate in &job.rates {
                    // Inject the line's pricing inputs so matrix expressions
                    // can key on them: loan.rate, loan.scenario.lockTermDays.
                    let mut ctx = loan.clone();
                    ctx["loan"]["rate"] = json!(rate);
                    ctx["scenario"]["lockTermDays"] = json!(job.term);
                    let mut adjustments = Vec::new();
                    let mut total = 0.0;
                    for key in &job.matrix_keys {
                        let mspan = tracing::info_span!(
                            "eval.matrix",
                            product = %job.product.code,
                            matrix = %key,
                            rate,
                            ok = tracing::field::Empty,
                            value = tracing::field::Empty
                        );
                        let _mg = mspan.enter();
                        match rt.block_on(engine.evaluate_serialized(key, ctx.clone().into(), opts))
                        {
                            Ok(out) => {
                                let result = &out["result"];
                                let value = result.get("value").and_then(Value::as_f64);
                                mspan.record("ok", true);
                                if let Some(v) = value {
                                    mspan.record("value", v);
                                    total += v;
                                }
                                adjustments.push(AdjustmentLine {
                                    matrix: key.clone(),
                                    value,
                                    adjustments: result
                                        .get("adjustments")
                                        .and_then(Value::as_array)
                                        .cloned()
                                        .unwrap_or_default(),
                                    performance: out
                                        .get("performance")
                                        .and_then(Value::as_str)
                                        .map(str::to_string),
                                    trace: out.get("trace").cloned(),
                                    error: None,
                                });
                            }
                            Err(e) => {
                                mspan.record("ok", false);
                                adjustments.push(AdjustmentLine {
                                    matrix: key.clone(),
                                    value: None,
                                    adjustments: vec![],
                                    performance: None,
                                    trace: None,
                                    error: Some(e.to_string()),
                                });
                            }
                        }
                    }
                    let base = job.sheet.price(*rate, job.term).unwrap_or(0.0);
                    lines.push(PricingLine {
                        rate: *rate,
                        lock_term_days: job.term,
                        base_price: base,
                        adjustment_total: total,
                        price: base + total,
                        adjustments,
                    });
                }
                PricedProduct {
                    product: job.product,
                    rate_sheet: job.sheet_key,
                    lines,
                }
            })
        })
        .collect();

    let mut priced = Vec::new();
    for join in futures::future::join_all(handles).await {
        match join {
            Ok(p) => priced.push(p),
            Err(e) => unpriced.push(UnpricedProduct {
                product: ProductRef {
                    code: "unknown".into(),
                    name: String::new(),
                },
                reason: format!("eval task failed: {e}"),
            }),
        }
    }
    priced.sort_by(|a, b| a.product.code.cmp(&b.product.code));

    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    tracing::Span::current()
        .record("priced", priced.len())
        .record("unpriced", unpriced.len())
        .record("elapsed_ms", elapsed_ms);
    Ok(PricingReport {
        revision: match revision {
            EvalRevision::Latest => "latest".to_string(),
            EvalRevision::Draft => "draft".to_string(),
            EvalRevision::AsOf(ts) => ts.to_rfc3339(),
        },
        priced,
        unpriced,
        elapsed_ms,
    })
}
