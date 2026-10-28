//! OIDC bearer-token machinery: JWT validation against a JWKS source,
//! DB-backed role resolution, and the request `Principal` extractor.
//! Route enforcement lands in MPE-99 — this module ships machinery plus
//! `/api/auth/whoami` as the end-to-end proof.

mod principal;
mod roles;
mod validator;

#[doc(hidden)]
pub mod testkit;

use std::sync::Arc;

use anyhow::Context;
use sqlx::PgPool;

pub use principal::Principal;
pub use roles::{Role, RoleResolver};
pub use validator::{AuthError, JwtValidator, TokenClaims};

use crate::config::Config;

/// Everything a request needs to turn a bearer token into a `Principal`:
/// signature validation plus the `role_grants` lookup.
pub struct AuthContext {
    pub validator: JwtValidator,
    pub roles: RoleResolver,
}

/// Build the auth context when `OIDC_ISSUER` is configured; `None` keeps the
/// API runnable without an IdP (auth extractors then fail with an error).
/// The initial key fetch retries briefly — the dev IdP may still be starting.
pub async fn init(cfg: &Config, pool: PgPool) -> anyhow::Result<Option<Arc<AuthContext>>> {
    let Some(issuer) = cfg.oidc_issuer.clone() else {
        tracing::warn!("OIDC_ISSUER unset — auth context disabled");
        return Ok(None);
    };

    let mut last_err = None;
    for attempt in 1..=10 {
        match JwtValidator::connect(&issuer, &cfg.oidc_audiences, cfg.oidc_jwks_url.as_deref())
            .await
        {
            Ok(validator) => {
                tracing::info!(%issuer, "OIDC validator ready");
                return Ok(Some(Arc::new(AuthContext {
                    validator,
                    roles: RoleResolver::new(pool),
                })));
            }
            Err(e) => {
                last_err = Some(e);
                if attempt < 10 {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("unknown error")))
        .context("OIDC discovery/JWKS fetch")
}
