//! Back-office admin API (`/v1/*`): JWT auth with server-side RBAC, the
//! admin journal (audit log, 4-eyes approvals, idempotency), live updates over
//! server-sent events and CORS. Endpoints mirror `apps/backoffice/API.md`.
//!
//! Engine state is only changed through journaled [`oms::Command`]s sent to
//! the single writer; admin-only state (audit, approvals, credit, KYC, users,
//! settings) lives in the admin journal (see [`store`]). Both replay
//! deterministically.

pub mod auth;
mod routes;
pub mod seed;
pub mod store;
mod stream;
pub mod views;

use crate::EngineHandle;
use auth::{Actor, Authenticator, Role};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use store::AdminStore;
use tokio::sync::{broadcast, Mutex};
use tower_http::cors::{AllowOrigin, CorsLayer};

#[derive(Clone, Debug)]
pub struct AdminConfig {
    /// Directory of the admin journal (normally the engine data dir).
    pub data_dir: PathBuf,
    /// `None`: no CORS headers (same origin only); `["*"]`: any origin.
    pub cors_origins: Option<Vec<String>>,
    /// Interval of the engine-change ticker that feeds the live stream.
    pub live_interval_ms: u64,
}

impl AdminConfig {
    pub fn new(data_dir: impl Into<PathBuf>) -> AdminConfig {
        AdminConfig {
            data_dir: data_dir.into(),
            cors_origins: None,
            live_interval_ms: 1_000,
        }
    }

    /// Reads `CORE_CORS_ORIGINS` (comma separated, `*` for any).
    pub fn with_env(mut self) -> AdminConfig {
        if let Ok(v) = std::env::var("CORE_CORS_ORIGINS") {
            let list: Vec<String> = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !list.is_empty() {
                self.cors_origins = Some(list);
            }
        }
        self
    }
}

#[derive(Clone)]
pub struct AdminCtx {
    pub engine: EngineHandle,
    pub store: Arc<Mutex<AdminStore>>,
    pub auth: Arc<Authenticator>,
    /// Invalidation hints for the live stream (JSON `{"topics": [...]}`).
    pub live: broadcast::Sender<String>,
}

impl AdminCtx {
    /// Publishes live-update topics (names of `AdminApi` methods).
    pub fn notify(&self, topics: &[&str]) {
        let _ = self.live.send(json!({ "topics": topics }).to_string());
    }
}

/// `{ "error": { "code", "message", "permission"? } }`
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub permission: Option<&'static str>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            code,
            message: message.into(),
            permission: None,
        }
    }
    pub fn bad(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, "invalid", message)
    }
    pub fn not_found(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, "not_found", message)
    }
    pub fn internal(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }
    pub fn unauthorized(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }
    pub fn forbidden(permission: &'static str) -> ApiError {
        ApiError {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: format!("missing permission {permission}"),
            permission: Some(permission),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut err = json!({ "code": self.code, "message": self.message });
        if let Some(p) = self.permission {
            err["permission"] = json!(p);
        }
        (self.status, Json(json!({ "error": err }))).into_response()
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> ApiError {
        ApiError::internal(format!("admin journal: {e}"))
    }
}

pub type ApiResult<T = Value> = Result<Json<T>, ApiError>;

pub fn need(actor: &Actor, permission: &'static str) -> Result<(), ApiError> {
    if actor.can(permission) {
        Ok(())
    } else {
        Err(ApiError::forbidden(permission))
    }
}

fn bearer(parts: &Parts) -> Option<&str> {
    parts
        .headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

impl FromRequestParts<AdminCtx> for Actor {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, ctx: &AdminCtx) -> Result<Actor, ApiError> {
        let token = bearer(parts).ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
        ctx.auth
            .verify(token)
            .map_err(|e| ApiError::unauthorized(format!("invalid token: {}", e.0)))
    }
}

/// The legacy engine routes (`/accounts`, `/commands`, ...) accept raw
/// engine commands, so they require the `admin` role; `/health` stays open.
async fn legacy_guard(State(ctx): State<AdminCtx>, req: Request, next: Next) -> Response {
    if req.uri().path() == "/health" {
        return next.run(req).await;
    }
    let (mut parts, body) = req.into_parts();
    match Actor::from_request_parts(&mut parts, &ctx).await {
        Ok(a) if a.role == Role::Admin => next.run(Request::from_parts(parts, body)).await,
        Ok(_) => ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "legacy engine routes require the admin role",
        )
        .into_response(),
        Err(e) => e.into_response(),
    }
}

fn cors(origins: &[String]) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::HeaderName::from_static("idempotency-key"),
            header::HeaderName::from_static("x-actor-hint"),
        ])
        .max_age(std::time::Duration::from_secs(600));
    if origins.iter().any(|o| o == "*") {
        layer.allow_origin(AllowOrigin::any())
    } else {
        let list: Vec<HeaderValue> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        layer.allow_origin(AllowOrigin::list(list))
    }
}

/// The full HTTP app: `/health`, `/auth/dev-token`, `/v1/*` and the legacy
/// engine routes. Spawns the live-update ticker on the current runtime.
pub fn app(engine: EngineHandle, auth: Authenticator, cfg: AdminConfig) -> std::io::Result<Router> {
    let store = AdminStore::open(&cfg.data_dir)?;
    let (live, _) = broadcast::channel(256);
    let ctx = AdminCtx {
        engine: engine.clone(),
        store: Arc::new(Mutex::new(store)),
        auth: Arc::new(auth),
        live,
    };
    stream::spawn_ticker(ctx.clone(), cfg.live_interval_ms);
    let legacy =
        crate::router(engine).layer(middleware::from_fn_with_state(ctx.clone(), legacy_guard));
    let mut app = routes::router().with_state(ctx).merge(legacy);
    if let Some(o) = &cfg.cors_origins {
        app = app.layer(cors(o));
    }
    Ok(app)
}
