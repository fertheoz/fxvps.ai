//! Back-office admin API (`/v1/*`): JWT auth with server-side RBAC, the
//! admin journal (audit log, 4-eyes approvals, idempotency), live updates over
//! server-sent events and CORS. Endpoints mirror `apps/backoffice/API.md`.
//!
//! Engine state is only changed through journaled [`oms::Command`]s sent to
//! the single writer; admin-only state (audit, approvals, credit, KYC, users,
//! settings) lives in the admin journal (see [`store`]). Both replay
//! deterministically.

pub mod activity;
pub mod alerts;
pub mod auth;
mod bridge_admin;
mod copy_admin;
mod econ_admin;
pub mod econ_calendar;
pub mod lp_poll;
mod routes;
pub mod seed;
pub mod statement;
pub mod store;
mod stream;
pub mod usdt_watch;
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
    /// FIX session table: of an in-process fix-gateway (`GatewayHandle::status_source`)
    /// or polled from a remote one ([`lp_poll::spawn`], `CORE_LP_STATUS_URL`).
    /// `None`: `/v1/lp/sessions` returns an empty list.
    pub lp_status: Option<LpStatus>,
    /// Managed LP config of a fix-gateway (`GET|PUT <url>/config`, bearer token);
    /// `None`: `/v1/lp/config` answers 404.
    pub lp_admin: Option<LpAdmin>,
    /// Multi-LP aggregator of an in-process stack; `None`: `/v1/lp/aggregation` answers 404.
    pub agg: Option<Arc<crate::lp_agg::Aggregator>>,
    /// Account name map of the in-process stack (client self-service routes).
    pub names: Option<crate::api::AccountNames>,
    /// Statement mailer; `None`: from the environment ([`statement::Mailer::from_env`]).
    pub mailer: Option<Arc<statement::Mailer>>,
    /// hazine.io payment links for USDT deposits (`HAZINE_PAYLINK_KEY`); `None`: manual.
    pub hazine: Option<usdt_watch::Hazine>,
    /// fix-gateway wire log (`<store_dir>/fixlog`, `CORE_FIX_LOG_DIR`), read-only.
    pub fix_log_dir: PathBuf,
    /// Connection / auth event log shared with the client gateway (parça 10a);
    /// `None`: a private, always-empty log.
    pub activity: Option<Arc<activity::ActivityLog>>,
}

/// fix-gateway admin endpoint (`FIX_ADMIN_TOKEN` on the gateway side).
#[derive(Clone, Debug)]
pub struct LpAdmin {
    /// Base URL, e.g. `http://127.0.0.1:9890`.
    pub url: String,
    pub token: String,
}

/// Shared FIX session table written by fix-gateway.
pub type LpStatus = Arc<std::sync::RwLock<Vec<fix_gateway::SessionStatus>>>;

impl AdminConfig {
    pub fn new(data_dir: impl Into<PathBuf>) -> AdminConfig {
        AdminConfig {
            data_dir: data_dir.into(),
            cors_origins: None,
            live_interval_ms: 1_000,
            lp_status: None,
            lp_admin: None,
            agg: None,
            names: None,
            mailer: None,
            hazine: None,
            fix_log_dir: PathBuf::from("/var/lib/fix-gateway/fixlog"),
            activity: None,
        }
    }

    /// Reads `CORE_CORS_ORIGINS` (comma separated). `*` (any origin) is only
    /// accepted together with dev auth (G15); otherwise startup fails.
    pub fn with_env(mut self, dev_auth: bool) -> Result<AdminConfig, String> {
        if let Ok(v) = std::env::var("CORE_FIX_LOG_DIR") {
            self.fix_log_dir = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("CORE_CORS_ORIGINS") {
            let list: Vec<String> = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if list.iter().any(|o| o == "*") && !dev_auth {
                return Err(
                    "CORE_CORS_ORIGINS=* is only allowed with CORE_DEV_AUTH=1; list explicit origins"
                        .into(),
                );
            }
            if !list.is_empty() {
                self.cors_origins = Some(list);
            }
        }
        self.hazine = usdt_watch::Hazine::from_env();
        Ok(self)
    }
}

#[derive(Clone)]
pub struct AdminCtx {
    pub fix_log_dir: PathBuf,
    /// Connection / auth events of the client gateway (parça 10a).
    pub activity: Arc<activity::ActivityLog>,
    pub engine: EngineHandle,
    pub store: Arc<Mutex<AdminStore>>,
    pub auth: Arc<Authenticator>,
    /// Invalidation hints for the live stream (JSON `{"topics": [...]}`).
    pub live: broadcast::Sender<String>,
    /// Single-use SSE tickets (`POST /v1/stream/ticket`).
    pub tickets: Arc<stream::Tickets>,
    pub lp_status: Option<LpStatus>,
    pub lp_admin: Option<LpAdmin>,
    pub agg: Option<Arc<crate::lp_agg::Aggregator>>,
    /// Operational alerts (stage 9).
    pub alerts: Arc<alerts::AlertBook>,
    /// External account id -> engine number (client self-service, stage 12).
    pub names: Option<crate::api::AccountNames>,
    /// Engine data directory (KYC documents live under `kyc/`).
    pub data_dir: PathBuf,
    /// Read replica for heavy reports (stage 14); `None` = reports run on the writer thread.
    pub replica: Option<Arc<tokio::sync::Mutex<crate::replica::Replica>>>,
    pub http: reqwest::Client,
    /// SMTP channel of the monthly statement e-mail; `None`: not configured.
    pub mailer: Option<Arc<statement::Mailer>>,
    /// hazine.io payment links for USDT deposits (`usdt_watch`).
    pub hazine: Option<usdt_watch::Hazine>,
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
    if !actor.can(permission) {
        return Err(ApiError::forbidden(permission));
    }
    if auth::needs_mfa(permission) && !actor.mfa_ok {
        return Err(ApiError {
            status: StatusCode::FORBIDDEN,
            code: "mfa_required",
            message: format!("{permission} requires a multi-factor login"),
            permission: Some(permission),
        });
    }
    Ok(())
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

/// A trading client calling the self-service routes (`/v1/client/*`):
/// identity token with an `accounts` claim, resolved to engine logins.
#[derive(Clone, Debug)]
pub struct ClientActor {
    pub sub: String,
    pub name: String,
    pub account_ids: Vec<String>,
    /// (external id, engine login) for the accounts known to this engine.
    pub logins: Vec<(String, u64)>,
    /// API-key token (`read` or `trade`): may look here; orders go through
    /// the client gateway. See [`ClientActor::interactive`].
    pub api_key: bool,
}

impl ClientActor {
    /// Refuses API-key tokens (`403`): funding requests, KYC documents, IB
    /// links and copy subscriptions need an interactive login, so a leaked
    /// bot key can never ask for a withdrawal.
    pub fn interactive(&self) -> Result<(), ApiError> {
        if self.api_key {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "api_key_forbidden",
                "API keys cannot do this; sign in to the terminal",
            ));
        }
        Ok(())
    }
    pub fn login_of(&self, external: &str) -> Option<u64> {
        self.logins
            .iter()
            .find(|(n, _)| n == external)
            .map(|(_, l)| *l)
    }
    pub fn owns(&self, login: u64) -> bool {
        self.logins.iter().any(|(_, l)| *l == login)
    }
}

impl FromRequestParts<AdminCtx> for ClientActor {
    type Rejection = ApiError;
    async fn from_request_parts(
        parts: &mut Parts,
        ctx: &AdminCtx,
    ) -> Result<ClientActor, ApiError> {
        let token = bearer(parts).ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
        let c = ctx
            .auth
            .verify_client(token)
            .map_err(|e| ApiError::unauthorized(format!("invalid token: {}", e.0)))?;
        let names = ctx
            .names
            .as_ref()
            .ok_or_else(|| ApiError::not_found("client self-service is not available here"))?;
        let logins = c
            .accounts
            .iter()
            .filter_map(|n| names.number(n).map(|l| (n.clone(), l)))
            .collect();
        Ok(ClientActor {
            name: c.name.unwrap_or_else(|| c.sub.clone()),
            sub: c.sub,
            account_ids: c.accounts,
            logins,
            api_key: c.api_key,
        })
    }
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
        Ok(a) if a.role == Role::Admin && a.mfa_ok => {
            next.run(Request::from_parts(parts, body)).await
        }
        Ok(_) => ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "legacy engine routes require the admin role (with MFA when required)",
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
    // The saved aggregation policy outlives restarts.
    if let (Some(a), Some(c)) = (&cfg.agg, &store.state.aggregation) {
        a.set_config(c.clone());
    }
    let (live, _) = broadcast::channel(256);
    let ctx = AdminCtx {
        fix_log_dir: cfg.fix_log_dir.clone(),
        activity: cfg.activity.clone().unwrap_or_default(),
        engine: engine.clone(),
        store: Arc::new(Mutex::new(store)),
        auth: Arc::new(auth),
        live,
        tickets: Arc::default(),
        lp_status: cfg.lp_status.clone(),
        lp_admin: cfg.lp_admin.clone(),
        agg: cfg.agg.clone(),
        alerts: Arc::default(),
        names: cfg.names.clone(),
        data_dir: cfg.data_dir.clone(),
        replica: match crate::replica::Replica::open(&cfg.data_dir) {
            Ok(r) => Some(Arc::new(tokio::sync::Mutex::new(r))),
            Err(e) => {
                tracing::warn!(error = %e, "report replica unavailable; reports run on the writer thread");
                None
            }
        },
        http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default(),
        mailer: cfg
            .mailer
            .clone()
            .or_else(|| statement::Mailer::from_env().map(Arc::new)),
        hazine: cfg.hazine.clone(),
    };
    stream::spawn_ticker(ctx.clone(), cfg.live_interval_ms);
    alerts::spawn(ctx.clone());
    statement::spawn(ctx.clone());
    usdt_watch::spawn(ctx.clone());
    let legacy =
        crate::router(engine).layer(middleware::from_fn_with_state(ctx.clone(), legacy_guard));
    let mut app = routes::router()
        .layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024))
        .with_state(ctx)
        .merge(legacy);
    if let Some(o) = &cfg.cors_origins {
        app = app.layer(cors(o));
    }
    Ok(app)
}

#[cfg(test)]
mod cors_env_tests {
    use super::AdminConfig;

    #[test]
    fn wildcard_cors_requires_dev_auth() {
        // Single test touches the env var to avoid races between tests.
        std::env::set_var("CORE_CORS_ORIGINS", "*");
        assert!(AdminConfig::new("/tmp").with_env(false).is_err());
        assert!(AdminConfig::new("/tmp").with_env(true).is_ok());
        std::env::set_var("CORE_CORS_ORIGINS", "https://admin.example");
        let cfg = AdminConfig::new("/tmp").with_env(false).unwrap();
        assert_eq!(
            cfg.cors_origins,
            Some(vec!["https://admin.example".to_string()])
        );
        std::env::remove_var("CORE_CORS_ORIGINS");
    }
}
