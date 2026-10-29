//! Per-route-group role enforcement. Each route bucket in `api::router`
//! carries a `route_layer` built on `role_gate` with its accepted role set —
//! the gate authenticates (401), checks roles (403), and stashes the
//! `Principal` in request extensions so handler extractors reuse it instead
//! of re-validating the token.
//!
//! Denials differ by kind: an authenticated caller with insufficient role
//! gets an attributed `auth.denied` audit row; an unauthenticated request
//! only logs — writing audit rows for bad/absent tokens is an unbounded
//! growth DoS vector.

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use serde_json::json;

use crate::api::AppState;
use crate::audit;
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
    let principal = match Principal::authenticate(&state, req.headers()).await {
        Ok(p) => p,
        Err(e) => {
            if let AppError::Unauthorized(m) = &e {
                tracing::warn!(error = %m, "unauthenticated request rejected");
            }
            return Err(e);
        }
    };
    if !roles.is_empty() && !principal.has_any(roles) {
        // Owned copies: a &Request borrow across the audit await would make
        // the gate future non-Send (axum bodies are !Sync).
        let method = req.method().as_str().to_string();
        let path = req.uri().path().to_string();
        if let Err(e) = audit_denied(&state, &principal, roles, method, path).await {
            // Never let an audit failure mask the denial itself.
            tracing::error!(error = %e, "auth.denied audit write failed");
        }
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

/// Persist an `auth.denied` chain entry for an authenticated-but-insufficient
/// caller. Its own transaction — nothing else is mutating here.
async fn audit_denied(
    state: &AppState,
    principal: &Principal,
    roles: &[Role],
    method: String,
    path: String,
) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    audit::record(
        &mut tx,
        principal,
        audit::Event {
            action: audit::action::AUTH_DENIED,
            artifact_table: None,
            artifact_key: None,
            detail: json!({
                "method": method,
                "path": path,
                "required": roles
                    .iter()
                    .map(|r| format!("{r:?}").to_lowercase())
                    .collect::<Vec<_>>(),
            }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
