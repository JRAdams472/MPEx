//! Authorization roles and the `role_grants` lookup. Role assignment lives
//! in the MPEx database — the IdP never needs to know MPEx roles.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Serialize;
use sqlx::PgPool;
use tokio::sync::Mutex;

/// Coarse-grained roles: `Viewer < Pricer < Admin` for humans; `Service` is
/// standalone — machine tokens (client-credentials) may evaluate and read
/// products only, never browse or mutate admin artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Pricer,
    Admin,
    Service,
}

impl Role {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "viewer" => Self::Viewer,
            "pricer" => Self::Pricer,
            "admin" => Self::Admin,
            "service" => Self::Service,
            _ => return None,
        })
    }

    /// Which granted roles satisfy a `required` level.
    pub fn satisfied_by(required: Self) -> &'static [Self] {
        match required {
            Self::Viewer => &[Self::Viewer, Self::Pricer, Self::Admin],
            Self::Pricer => &[Self::Pricer, Self::Admin],
            Self::Admin => &[Self::Admin],
            Self::Service => &[Self::Service],
        }
    }
}

/// Resolves `role_grants` rows for a token's identity keys, briefly cached —
/// role changes propagate within ~60s across instances (grant mutations
/// invalidate the local cache immediately). Identity keys are typed pairs:
/// `("sub", sub)`, `("email", email)` (dex static users identify by email;
/// `sub` is an opaque encoded id), and `("aud", client_id)` for each
/// audience (client-credentials tokens identify by client_id in `aud`).
pub struct RoleResolver {
    pool: PgPool,
    cache: Mutex<HashMap<String, (HashSet<Role>, Instant)>>,
}

const CACHE_TTL: Duration = Duration::from_secs(60);

impl RoleResolver {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// `keys` are `(subject_type, subject)` pairs matching the
    /// `role_grants.subject_type` vocabulary ('sub' | 'email' | 'aud').
    pub async fn roles_for(&self, keys: &[(String, String)]) -> Result<HashSet<Role>, sqlx::Error> {
        let cache_key = keys
            .iter()
            .map(|(t, s)| format!("{t}\u{1e}{s}"))
            .collect::<Vec<_>>()
            .join("\u{1f}");
        {
            let cache = self.cache.lock().await;
            if let Some((roles, at)) = cache.get(&cache_key) {
                if at.elapsed() < CACHE_TTL {
                    return Ok(roles.clone());
                }
            }
        }

        let types: Vec<&str> = keys.iter().map(|(t, _)| t.as_str()).collect();
        let subjects: Vec<&str> = keys.iter().map(|(_, s)| s.as_str()).collect();
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT role FROM role_grants
             WHERE (subject_type, subject) IN
                   (SELECT * FROM unnest($1::text[], $2::text[]))",
        )
        .bind(&types)
        .bind(&subjects)
        .fetch_all(&self.pool)
        .await?;
        let roles: HashSet<Role> = rows.iter().filter_map(|r| Role::parse(r)).collect();

        self.cache
            .lock()
            .await
            .insert(cache_key, (roles.clone(), Instant::now()));
        Ok(roles)
    }

    /// Drop every cached resolution — grant mutations call this post-commit
    /// so the new grants take effect on the very next request on this
    /// instance. Other replicas still converge within `CACHE_TTL`.
    pub async fn invalidate_all(&self) {
        self.cache.lock().await.clear();
    }
}
