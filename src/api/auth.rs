use axum::Json;
use serde_json::{json, Value};

use crate::auth::Principal;

/// Proves the auth machinery end-to-end: a valid bearer token yields the
/// caller's claims + resolved roles; anything else is a 401.
pub async fn whoami(principal: Principal) -> Json<Value> {
    let mut roles: Vec<_> = principal.roles.iter().collect();
    roles.sort();
    Json(json!({
        "sub": principal.sub,
        "email": principal.email,
        "roles": roles,
    }))
}
