//! Per-route-group role enforcement. Each route bucket in `api::router`
//! carries a `route_layer` built on `role_gate` with its accepted role set —
//! the gate authenticates (401), checks roles (403), and stashes the
//! `Principal` in request extensions so handler extractors reuse it instead
//! of re-validating the token.

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::api::AppState;
use crate::auth::{Principal, Role};
use crate::error::AppError;

/// Role gate for a bucket of routes. `roles` is the accepted set — an empty
/// slice means "any authenticated principal" (e.g. whoami accepts callers
/// with no grants).
pub async fn role_gate(
    State((state, roles)): State<(AppState, &'static [Role])>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let principal = Principal::authenticate(&state, req.headers()).await?;
    if !roles.is_empty() && !principal.has_any(roles) {
        return Err(AppError::Forbidden(format!(
            "insufficient role: {} required",
            roles
                .iter()
                .map(|r| format!("{r:?}").to_lowercase())
                .collect::<Vec<_>>()
                .join(" or ")
        )));
    }
    req.extensions_mut().insert(principal);
    Ok(next.run(req).await)
}
