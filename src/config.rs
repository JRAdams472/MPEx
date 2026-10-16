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
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://mpex:mpex@localhost:5432/mpex".into()),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            cors_origin: std::env::var("CORS_ORIGIN")
                .unwrap_or_else(|_| "http://localhost:3000".into()),
            otel_endpoint: std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok(),
            seq_url: std::env::var("SEQ_URL").ok(),
            seq_api_key: std::env::var("SEQ_API_KEY").ok(),
        }
    }
}
