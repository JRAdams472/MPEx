use axum::Json;
use serde_json::{json, Value};

use crate::auth::Principal;

/// Proves the auth machinery end-to-end: a valid bearer token yields the
/// caller's claims + resolved roles; anything else is a 401.
pub async fn whoami(principal: Principal) -> Json<Value> {
    let mut roles: Vec<_> = principal.roles.iter().collect();
    roles.sort();
    // NULL = unrestricted; a sorted array lists the lanes the caller may
    // evaluate in (MPE-153). The sandbox hints at this before a 403.
    let permitted_envs = principal.permitted_envs.as_ref().map(|envs| {
        let mut v: Vec<_> = envs.iter().collect();
        v.sort();
        v
    });
    Json(json!({
        "sub": principal.sub,
        "email": principal.email,
        "roles": roles,
        "permittedEnvs": permitted_envs,
    }))
}
