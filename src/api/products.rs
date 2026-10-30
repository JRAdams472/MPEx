use axum::extract::State;
use axum::Json;
use serde_json::Value;

use crate::api::AppState;
use crate::error::AppError;

type ProductRow = (
    String,
    String,
    String,
    String,
    i32,
    String,
    bool,
    Option<String>,
    Option<String>,
    f64,
);

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let rows = sqlx::query_as::<_, ProductRow>(
        "SELECT code, name, loan_type, amortization, term_months, description, active,
                mnd_label, base_product_code, rate_adjustment
         FROM products ORDER BY code",
    )
    .fetch_all(&state.pool)
    .await?;

    let products: Vec<Value> = rows
        .into_iter()
        .map(
            |(
                code,
                name,
                loan_type,
                amortization,
                term_months,
                description,
                active,
                mnd_label,
                base_product_code,
                rate_adjustment,
            )| {
                serde_json::json!({
                    "code": code,
                    "name": name,
                    "loanType": loan_type,
                    "amortization": amortization,
                    "termMonths": term_months,
                    "description": description,
                    "active": active,
                    "mndLabel": mnd_label,
                    "baseProductCode": base_product_code,
                    "rateAdjustment": rate_adjustment,
                })
            },
        )
        .collect();
    Ok(Json(serde_json::json!({ "products": products })))
}
