mod audit;
mod auth;
mod decisions;
mod evaluate;
mod expressions;
mod grants;
mod matrices;
mod products;
mod publications;
mod rate_index;
mod rate_sheets;
pub(crate) mod validate;

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{delete, get, patch, post, put};
use axum::{middleware, Json, Router};
use std::sync::Arc;

use serde_json::{json, Value};
use sqlx::PgPool;

use crate::auth::{role_gate, Role};
use crate::error::AppError;
use crate::rules::snapshot::SnapshotStore;

/// Admin listing revision selector: None = working drafts, Some(ts) = the
/// published snapshot as of that timestamp ("draft" also maps to None).
fn parse_admin_as_of(
    as_of: Option<&str>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    match as_of {
        None | Some("draft") => Ok(None),
        Some(ts) => chrono::DateTime::parse_from_rfc3339(ts)
            .map(|t| Some(t.with_timezone(&chrono::Utc)))
            .map_err(|_| {
                AppError::BadRequest(format!("asOf must be 'draft' or RFC3339, got {ts:?}"))
            }),
    }
}

/// `?env=` on admin list views: omitted = the dev working lane (MPE-153).
fn parse_env(env: Option<&str>) -> Result<crate::rules::loader::Env, AppError> {
    env.map(|e| {
        crate::rules::loader::Env::parse(e).ok_or_else(|| {
            AppError::BadRequest(format!("env must be one of dev|qa|uat|prod: {e:?}"))
        })
    })
    .transpose()
    .map(|e| e.unwrap_or(crate::rules::loader::Env::Dev))
}

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub snapshots: Arc<SnapshotStore>,
    /// OIDC context — `None` keeps the API runnable without an IdP, but
    /// every gated route then rejects (no auth bypass: dev runs dex).
    pub auth: Option<Arc<crate::auth::AuthContext>>,
    /// Per-IP rate limits — `None` in tests (no throttling); `main` fills
    /// it from env. Applied *inside* `security_headers` so 429s carry the
    /// header baseline too.
    pub rate_limits: Option<Arc<crate::limits::EdgeLimits>>,
    /// MND rate-index fetch settings — `None` disables the preview/import
    /// endpoints; tests point it at a stub server.
    pub rate_index: Option<crate::rate_index::IndexSettings>,
}

/// Role buckets — each merged sub-router carries its own gate layer.
/// `Service` is standalone: it may evaluate + list products and nothing
/// else (it can't even reach viewer routes — `has` is hierarchical, not
/// a superset).
const EVAL_OR_PRODUCTS: &[Role] = &[Role::Viewer, Role::Service];
const VIEWER: &[Role] = &[Role::Viewer];
const PRICER: &[Role] = &[Role::Pricer];
const ADMIN: &[Role] = &[Role::Admin];
/// The audit trail read surface — admins plus the read-only auditor role.
const AUDIT_READERS: &[Role] = &[Role::Admin, Role::Auditor];

pub fn router(state: AppState) -> Router {
    // Authenticated callers (viewer+ or service): evaluation + products.
    let eval = Router::new()
        .route("/api/evaluate/eligibility", post(evaluate::eligibility))
        .route("/api/evaluate/pricing", post(evaluate::pricing))
        .route("/api/products", get(products::list))
        .route_layer(middleware::from_fn_with_state(
            (state.clone(), EVAL_OR_PRODUCTS),
            role_gate,
        ));

    // viewer+ routes: validation/simulation and every admin GET.
    let viewer = Router::new()
        .route(
            "/api/validate/expression",
            post(evaluate::validate_expression),
        )
        .route("/api/simulate/matrix", post(matrices::simulate))
        .route("/api/decisions", get(decisions::list))
        .route("/api/decisions/{key}", get(decisions::get))
        .route("/api/expressions", get(expressions::list))
        .route("/api/expressions/{key}", get(expressions::get))
        .route("/api/matrices", get(matrices::list))
        .route("/api/matrices/{key}", get(matrices::get))
        .route("/api/rate-sheets", get(rate_sheets::list))
        .route("/api/rate-sheets/{key}", get(rate_sheets::get))
        .route("/api/rate-index", get(rate_index::get))
        .route("/api/publications", get(publications::list))
        .route("/api/publications/{id}", get(publications::get))
        .route_layer(middleware::from_fn_with_state(
            (state.clone(), VIEWER),
            role_gate,
        ));

    // pricer+ artifact mutations (GETs on the same paths stay viewer+).
    let pricer = Router::new()
        .route(
            "/api/decisions/{key}",
            put(decisions::put).delete(decisions::delete),
        )
        .route(
            "/api/expressions/{key}",
            put(expressions::put).delete(expressions::delete),
        )
        .route(
            "/api/matrices/{key}",
            put(matrices::put).delete(matrices::delete),
        )
        .route("/api/matrices/{key}/cells", patch(matrices::patch_cells))
        .route(
            "/api/rate-sheets/{key}",
            put(rate_sheets::put).delete(rate_sheets::delete),
        )
        .route(
            "/api/rate-sheets/{key}/cells",
            patch(rate_sheets::patch_cells),
        )
        .route_layer(middleware::from_fn_with_state(
            (state.clone(), PRICER),
            role_gate,
        ));

    // Audit trail read surface (MPE-172): admins + the read-only auditor
    // role. Kept in its own bucket so auditor never widens the admin gate.
    let audit = Router::new()
        .route("/api/audit", get(audit::list))
        .route("/api/audit/verify", get(audit::verify))
        .route_layer(middleware::from_fn_with_state(
            (state.clone(), AUDIT_READERS),
            role_gate,
        ));

    // admin-only: publishing stamps every changed draft; grants, promotions,
    // and rate-index imports are privileged mutations.
    let admin = Router::new()
        .route("/api/publications", post(publications::create))
        .route("/api/admin/grants", get(grants::list).put(grants::upsert))
        .route("/api/admin/grants/{id}", delete(grants::revoke))
        .route("/api/admin/promotions", post(publications::promote))
        .route("/api/admin/rate-index/preview", post(rate_index::preview))
        .route("/api/admin/rate-index/import", post(rate_index::import))
        .route(
            "/api/admin/products/{code}/rate-config",
            put(rate_index::put_rate_config),
        )
        .route_layer(middleware::from_fn_with_state(
            (state.clone(), ADMIN),
            role_gate,
        ));

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/health", get(health))
        // whoami authenticates via its own extractor — any valid token,
        // including zero-role principals.
        .route("/api/auth/whoami", get(auth::whoami))
        .merge(eval)
        .merge(viewer)
        .merge(pricer)
        .merge(audit)
        .merge(admin);

    // Rate limits sit inside the headers layer so 429s carry them too.
    let app = match &state.rate_limits {
        Some(l) => app.layer(middleware::from_fn_with_state(
            l.clone(),
            crate::limits::edge_limit,
        )),
        None => app,
    };

    app.layer(middleware::from_fn(security_headers))
        // Hard ceiling on request bodies; semantic caps in `validate` are
        // the finer bounds inside it.
        .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(state)
}

/// Baseline response hardening on every response, including gate rejections
/// and health checks. The API serves JSON only, so the CSP is a deny-all
/// document policy that also kills framing (`frame-ancestors` covers the
/// `X-Frame-Options` gaps in modern browsers).
async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'self'; frame-ancestors 'none'"),
    );
    h.insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"),
    );
    res
}

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn health(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(json!({ "status": "ok", "db": "up" })))
}
