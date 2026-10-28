//! The authenticated caller, extracted per request: validated token claims
//! plus resolved roles. Per-route role requirements layer on top in MPE-99.

use std::collections::HashSet;

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;

use crate::api::AppState;
use crate::auth::{AuthError, Role};
use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct Principal {
    pub sub: String,
    pub email: Option<String>,
    pub roles: HashSet<Role>,
}

impl Principal {
    /// True when any granted role satisfies `required` (viewer < pricer <
    /// admin; service is standalone).
    pub fn has(&self, required: Role) -> bool {
        self.roles
            .iter()
            .any(|r| Role::satisfied_by(required).contains(r))
    }

    /// True when the principal satisfies at least one of `required`
    /// (e.g. `eval` accepts `[Viewer, Service]`).
    pub fn has_any(&self, required: &[Role]) -> bool {
        required.iter().any(|&r| self.has(r))
    }
}

impl FromRequestParts<AppState> for Principal {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Some(auth) = &state.auth else {
            return Err(AppError::Internal(anyhow::anyhow!(
                "auth required but no OIDC context configured"
            )));
        };
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .ok_or_else(|| AppError::Unauthorized("missing bearer token".into()))?;
        let claims = auth.validator.validate(token).await.map_err(|e| match e {
            AuthError::Invalid(m) => AppError::Unauthorized(m),
            AuthError::UnknownKid => AppError::Unauthorized("unknown signing key".into()),
        })?;
        // Identity keys for grant lookup: opaque sub, human email, and the
        // client_id that dex stamps into `aud` on client-credentials tokens.
        let mut keys = vec![claims.sub.clone()];
        keys.extend(claims.email.iter().cloned());
        keys.extend(claims.audiences());
        let roles = auth.roles.roles_for(&keys).await?;
        Ok(Principal {
            sub: claims.sub,
            email: claims.email,
            roles,
        })
    }
}
