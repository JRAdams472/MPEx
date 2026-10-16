use axum::extract::State;
use axum::Json;
use serde_json::Value;

use crate::api::AppState;
use crate::error::AppError;

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let rows = sqlx::query_as::<_, (String, String, String, String, i32, String, bool)>(
        "SELECT code, name, loan_type, amortization, term_months, description, active \
         FROM products ORDER BY code",
    )
    .fetch_all(&state.pool)
    .await?;

    let products: Vec<Value> = rows
        .into_iter()
        .map(
            |(code, name, loan_type, amortization, term_months, description, active)| {
                serde_json::json!({
                    "code": code,
                    "name": name,
                    "loanType": loan_type,
                    "amortization": amortization,
                    "termMonths": term_months,
                    "description": description,
                    "active": active,
                })
            },
        )
        .collect();
    Ok(Json(serde_json::json!({ "products": products })))
}
