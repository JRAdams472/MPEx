#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    pub cors_origin: String,
    /// OTLP gRPC endpoint for trace export (e.g. http://jaeger:4317).
    /// Unset = tracing spans stay local to stdout.
    pub otel_endpoint: Option<String>,
    /// OTLP endpoint for log export (Seq ingest, e.g. http://seq:5341).
    /// Unset = stdout JSON only.
    pub seq_url: Option<String>,
    /// Optional `x-seq-apikey` header for the Seq ingest endpoint.
    pub seq_api_key: Option<String>,
    /// Expected JWT `iss` (e.g. `http://localhost:5556/dex` for the dev stack).
    /// Unset = no auth context: bearer-protected extractors reject with 500.
    pub oidc_issuer: Option<String>,
    /// Accepted JWT `aud` values (comma-separated env). Dex stamps `aud` with
    /// the *client id* — `mpex-web` for UI sessions, `mpex-svc` for service
    /// tokens — so the accepted set is the registered client list.
    pub oidc_audiences: Vec<String>,
    /// Explicit JWKS endpoint when it differs from the issuer's public
    /// discovery URL (e.g. `http://dex:5556/dex/keys` inside compose).
    /// Unset = resolve via `{issuer}/.well-known/openid-configuration`.
    pub oidc_jwks_url: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            // No baked-in default: a well-known credential in source is a
            // secret leak. Set DATABASE_URL (compose injects it from .env).
            database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            cors_origin: std::env::var("CORS_ORIGIN")
                .unwrap_or_else(|_| "http://localhost:3000".into()),
            otel_endpoint: std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok(),
            seq_url: std::env::var("SEQ_URL").ok(),
            seq_api_key: std::env::var("SEQ_API_KEY").ok(),
            oidc_issuer: std::env::var("OIDC_ISSUER").ok(),
            oidc_audiences: std::env::var("OIDC_AUDIENCE")
                .unwrap_or_else(|_| "mpex-api".into())
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            oidc_jwks_url: std::env::var("OIDC_JWKS_URL")
                .ok()
                .filter(|s| !s.is_empty()),
        }
    }
}
