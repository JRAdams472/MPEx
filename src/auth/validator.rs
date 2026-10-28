//! JWT validation against an issuer's JWKS. Two construction paths:
//! `connect` — production, fetches keys from an explicit JWKS URL or via
//! OIDC discovery; `for_test` — a static JWKS document, no network.
//! The algorithm allowlist is checked before any signature work so
//! `none`/symmetric-alg confusion attacks die at the header parse.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use anyhow::Context;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

/// Signature algorithms accepted from the issuer. Symmetric algs (HS*) and
/// `none` can never appear here.
const ALLOWED_ALGS: &[Algorithm] = &[Algorithm::RS256, Algorithm::ES256, Algorithm::EdDSA];

/// Minimum interval between JWKS refreshes triggered by an unknown `kid`
/// (issuer key rotation). Bounds refresh traffic on a kid-scan attempt.
const REFRESH_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub enum AuthError {
    /// Token failed validation — malformed, bad signature, expired,
    /// wrong iss/aud, or disallowed alg.
    Invalid(String),
    /// Signing key not found even after a JWKS refresh.
    UnknownKid,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(m) => write!(f, "invalid token: {m}"),
            Self::UnknownKid => write!(f, "unknown signing key"),
        }
    }
}
impl std::error::Error for AuthError {}

/// Validated token claims. `iss`/`aud`/`exp`/`nbf` are enforced by the
/// validator itself — this carries identity forward to role resolution.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenClaims {
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl TokenClaims {
    /// `aud` may be a string or an array — normalize for role-grant lookup
    /// (client-credentials tokens identify by client_id, which arrives here).
    pub fn audiences(&self) -> Vec<String> {
        match self.extra.get("aud") {
            Some(serde_json::Value::String(s)) => vec![s.clone()],
            Some(serde_json::Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            _ => vec![],
        }
    }
}

struct KeyCache {
    by_kid: HashMap<String, DecodingKey>,
    refreshed_at: Instant,
}

pub struct JwtValidator {
    expected_iss: String,
    expected_aud: Vec<String>,
    client: reqwest::Client,
    /// Explicit JWKS endpoint; `None` on the test path (no refresh possible).
    jwks_url: Option<String>,
    keys: RwLock<KeyCache>,
}

impl JwtValidator {
    /// Build a validator from either an explicit JWKS URL or OIDC
    /// discovery on the issuer. Fetches the key set eagerly — callers
    /// handle retry (see `auth::init`). `audiences` is the accepted set —
    /// dex stamps `aud` with the client id, so this is the registered
    /// client list (UI + service clients).
    pub async fn connect(
        issuer: &str,
        audiences: &[String],
        jwks_url: Option<&str>,
    ) -> anyhow::Result<Self> {
        let client = reqwest::Client::new();
        let jwks_url = match jwks_url {
            Some(u) => u.to_string(),
            None => {
                let discovery_url = format!(
                    "{}/.well-known/openid-configuration",
                    issuer.trim_end_matches('/')
                );
                let discovery: serde_json::Value = client
                    .get(&discovery_url)
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                discovery["jwks_uri"]
                    .as_str()
                    .context("discovery doc lacks jwks_uri")?
                    .to_string()
            }
        };
        let v = Self {
            expected_iss: issuer.into(),
            expected_aud: audiences.to_vec(),
            client,
            jwks_url: Some(jwks_url),
            keys: RwLock::new(KeyCache {
                by_kid: HashMap::new(),
                refreshed_at: Instant::now()
                    .checked_sub(REFRESH_INTERVAL)
                    .unwrap_or_else(Instant::now),
            }),
        };
        v.refresh().await?;
        Ok(v)
    }

    /// Static-JWKS validator for tests: no network, no refresh.
    pub fn for_test(issuer: &str, audiences: &[&str], jwks_json: &str) -> anyhow::Result<Self> {
        let jwks: JwkSet = serde_json::from_str(jwks_json)?;
        Ok(Self {
            expected_iss: issuer.into(),
            expected_aud: audiences.iter().map(|s| s.to_string()).collect(),
            client: reqwest::Client::new(),
            jwks_url: None,
            keys: RwLock::new(KeyCache {
                by_kid: decode_keys(&jwks),
                refreshed_at: Instant::now(),
            }),
        })
    }

    /// Validate a compact JWT: header alg allowlist → kid lookup (refresh
    /// once on miss) → signature + iss/aud/exp/nbf/sub checks.
    pub async fn validate(&self, token: &str) -> Result<TokenClaims, AuthError> {
        let header =
            decode_header(token).map_err(|e| AuthError::Invalid(format!("malformed: {e}")))?;
        if !ALLOWED_ALGS.contains(&header.alg) {
            return Err(AuthError::Invalid(format!(
                "alg {:?} not allowed",
                header.alg
            )));
        }
        let kid = header
            .kid
            .ok_or_else(|| AuthError::Invalid("missing kid".into()))?;

        let key = self.key_for(&kid).await.ok_or(AuthError::UnknownKid)?;

        let mut validation = Validation::new(header.alg);
        validation.leeway = 60;
        validation.set_issuer(&[&self.expected_iss]);
        validation.set_audience(&self.expected_aud);
        validation.required_spec_claims = ["exp", "iss", "aud", "sub"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        validation.validate_nbf = true;

        let data = decode::<TokenClaims>(token, &key, &validation)
            .map_err(|e| AuthError::Invalid(e.to_string()))?;
        Ok(data.claims)
    }

    /// Kid lookup; on a cache miss, refreshes the JWKS once (bounded by
    /// REFRESH_INTERVAL) and retries — issuer key rotation.
    async fn key_for(&self, kid: &str) -> Option<DecodingKey> {
        if let Some(k) = self.keys.read().ok()?.by_kid.get(kid) {
            return Some(k.clone());
        }
        {
            let cache = self.keys.read().ok()?;
            if self.jwks_url.is_none() || cache.refreshed_at.elapsed() < REFRESH_INTERVAL {
                return None;
            }
        }
        if self.refresh().await.is_ok() {
            self.keys.read().ok()?.by_kid.get(kid).cloned()
        } else {
            None
        }
    }

    async fn refresh(&self) -> anyhow::Result<()> {
        let Some(url) = &self.jwks_url else {
            return Ok(());
        };
        let jwks: JwkSet = self
            .client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut cache = self
            .keys
            .write()
            .map_err(|_| anyhow::anyhow!("jwks cache poisoned"))?;
        cache.by_kid = decode_keys(&jwks);
        cache.refreshed_at = Instant::now();
        Ok(())
    }
}

fn decode_keys(jwks: &JwkSet) -> HashMap<String, DecodingKey> {
    jwks.keys
        .iter()
        .filter_map(|j| {
            let kid = j.common.key_id.clone()?;
            let key = match &j.algorithm {
                AlgorithmParameters::RSA(p) => DecodingKey::from_rsa_components(&p.n, &p.e).ok()?,
                AlgorithmParameters::EllipticCurve(p) => {
                    DecodingKey::from_ec_components(&p.x, &p.y).ok()?
                }
                AlgorithmParameters::OctetKeyPair(p) => {
                    DecodingKey::from_ed_components(&p.x).ok()?
                }
                _ => return None,
            };
            Some((kid, key))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::testkit;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::{Map, Value};

    fn mint_claims(claims: Value, kid: Option<&str>, alg: Algorithm, secret: &str) -> String {
        let mut header = Header::new(alg);
        header.kid = kid.map(String::from);
        let key = match alg {
            Algorithm::RS256 => {
                EncodingKey::from_rsa_pem(testkit::PRIVATE_KEY_PEM.as_bytes()).unwrap()
            }
            Algorithm::HS256 => EncodingKey::from_secret(secret.as_bytes()),
            _ => unimplemented!(),
        };
        encode(&header, &claims, &key).unwrap()
    }

    fn good_claims() -> Map<String, Value> {
        let now = jsonwebtoken::get_current_timestamp() as i64;
        Map::from_iter([
            ("iss".into(), testkit::ISSUER.into()),
            ("aud".into(), testkit::AUDIENCE.into()),
            ("sub".into(), "user-1".into()),
            ("exp".into(), (now + 3600).into()),
            ("iat".into(), now.into()),
        ])
    }

    fn validator() -> JwtValidator {
        JwtValidator::for_test(testkit::ISSUER, &[testkit::AUDIENCE], testkit::JWKS_JSON).unwrap()
    }

    #[tokio::test]
    async fn valid_token_yields_claims() {
        let token = mint_claims(
            Value::Object(good_claims()),
            Some(testkit::KID),
            Algorithm::RS256,
            "",
        );
        let claims = validator().validate(&token).await.unwrap();
        assert_eq!(claims.sub, "user-1");
    }

    #[tokio::test]
    async fn wrong_issuer_rejected() {
        let mut claims = good_claims();
        claims.insert("iss".into(), "https://evil.example".into());
        let token = mint_claims(
            Value::Object(claims),
            Some(testkit::KID),
            Algorithm::RS256,
            "",
        );
        assert!(matches!(
            validator().validate(&token).await,
            Err(AuthError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn wrong_audience_rejected() {
        let mut claims = good_claims();
        claims.insert("aud".into(), "other-api".into());
        let token = mint_claims(
            Value::Object(claims),
            Some(testkit::KID),
            Algorithm::RS256,
            "",
        );
        assert!(matches!(
            validator().validate(&token).await,
            Err(AuthError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn expired_rejected() {
        let mut claims = good_claims();
        let now = jsonwebtoken::get_current_timestamp() as i64;
        claims.insert("exp".into(), (now - 3600).into());
        let token = mint_claims(
            Value::Object(claims),
            Some(testkit::KID),
            Algorithm::RS256,
            "",
        );
        assert!(matches!(
            validator().validate(&token).await,
            Err(AuthError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn symmetric_alg_rejected() {
        // HS256 signed with the "public" key material — classic alg confusion.
        let token = mint_claims(
            Value::Object(good_claims()),
            Some(testkit::KID),
            Algorithm::HS256,
            "whatever",
        );
        assert!(matches!(
            validator().validate(&token).await,
            Err(AuthError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn alg_none_rejected() {
        // Hand-craft {"alg":"none"} — jsonwebtoken refuses to even parse the
        // header (no `none` variant), so this dies as malformed.
        let b64 = |s: &str| {
            use base64::Engine;
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s)
        };
        let token = format!(
            "{}.{}.",
            b64(r#"{"alg":"none","typ":"JWT"}"#),
            b64(
                r#"{"iss":"https://test-issuer.invalid","aud":"mpex-api","sub":"u","exp":9999999999}"#
            )
        );
        assert!(matches!(
            validator().validate(&token).await,
            Err(AuthError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn unknown_kid_rejected() {
        let token = mint_claims(
            Value::Object(good_claims()),
            Some("no-such-key"),
            Algorithm::RS256,
            "",
        );
        assert!(matches!(
            validator().validate(&token).await,
            Err(AuthError::UnknownKid)
        ));
    }

    #[tokio::test]
    async fn missing_kid_rejected() {
        let token = mint_claims(Value::Object(good_claims()), None, Algorithm::RS256, "");
        assert!(matches!(
            validator().validate(&token).await,
            Err(AuthError::Invalid(_))
        ));
    }
}
