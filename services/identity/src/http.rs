//! HTTP API (axum). See the crate README for the endpoint reference.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, FromRequestParts, Query, Request, State};
use axum::http::header::{AUTHORIZATION, COOKIE, ORIGIN, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::cors::{AllowOrigin, CorsLayer};
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Uuid,
};

use crate::crypto::{self, random_id, random_token, sha256_hex};
use crate::mail::Mail;
use crate::store::{
    OneTimeToken, PasskeyRecord, RefreshToken, Rotation, StoreError, TokenKind, User,
};
use crate::{now, AccessClaims, App, ROLES};

/// Refresh token cookie (web clients).
pub const REFRESH_COOKIE: &str = "fxvps_rt";
/// Header a cookie-mode request must carry (forces a CORS preflight; CSRF defence
/// in depth on top of `SameSite=Strict` and the Origin allow list).
pub const CSRF_HEADER: &str = "x-fxvps-csrf";
const COOKIE_PATH: &str = "/v1/token";
const MFA_MAX_ATTEMPTS: i32 = 5;

// ---------------------------------------------------------------- errors

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        ApiError {
            status,
            code,
            message: message.into(),
        }
    }
    fn bad(code: &'static str, m: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, m)
    }
    fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "authentication required",
        )
    }
    fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", "insufficient role")
    }
    fn invalid_credentials() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "invalid email or password",
        )
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Conflict => Self::new(StatusCode::CONFLICT, "conflict", "already exists"),
            StoreError::NotFound => Self::new(StatusCode::NOT_FOUND, "not_found", "not found"),
            StoreError::Backend(m) => {
                tracing::error!(error = %m, "store");
                Self::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error",
                )
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "error": self.code, "message": self.message })),
        )
            .into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

// ---------------------------------------------------------------- extractors

/// Client IP: the socket peer, or (with `IDENTITY_TRUST_PROXY`) the
/// `X-Forwarded-For` entry added by the outermost trusted proxy.
#[derive(Clone, Debug)]
pub struct ClientIp(pub Option<String>);

/// The client address in `X-Forwarded-For` behind `hops` trusted proxies: each
/// proxy appends the address it received the request from, so the entry `hops`
/// positions from the right is the one written by the outermost trusted proxy.
/// Entries further left are client controlled and ignored. `None` when the
/// header is shorter than `hops` or the entry is not an IP address.
pub fn forwarded_client_ip(xff: &str, hops: usize) -> Option<String> {
    let parts: Vec<&str> = xff.split(',').map(str::trim).collect();
    let i = parts.len().checked_sub(hops.max(1))?;
    parts[i]
        .parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| ip.to_string())
}

fn client_ip(app: &App, headers: &HeaderMap, ext: &axum::http::Extensions) -> Option<String> {
    if app.cfg.trust_proxy {
        let xff: Vec<&str> = headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect();
        if let Some(ip) = forwarded_client_ip(&xff.join(","), app.cfg.trusted_proxy_hops) {
            return Some(ip);
        }
    }
    ext.get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string())
}

impl FromRequestParts<Arc<App>> for ClientIp {
    type Rejection = ApiError;
    async fn from_request_parts(p: &mut Parts, app: &Arc<App>) -> ApiResult<Self> {
        Ok(ClientIp(client_ip(app, &p.headers, &p.extensions)))
    }
}

/// A verified access token (`Authorization: Bearer`).
pub struct Authed(pub AccessClaims);

fn bearer(h: &HeaderMap) -> Option<&str> {
    h.get(AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ")
}

impl FromRequestParts<Arc<App>> for Authed {
    type Rejection = ApiError;
    async fn from_request_parts(p: &mut Parts, app: &Arc<App>) -> ApiResult<Self> {
        let t = bearer(&p.headers).ok_or_else(ApiError::unauthorized)?;
        app.verify_access(t)
            .map(Authed)
            .ok_or_else(ApiError::unauthorized)
    }
}

/// Caller of `/v1/admin/*`: an access token with the `admin` role, or the
/// configured service token (core-engine / back office).
pub struct Admin {
    pub actor: String,
}

impl FromRequestParts<Arc<App>> for Admin {
    type Rejection = ApiError;
    async fn from_request_parts(p: &mut Parts, app: &Arc<App>) -> ApiResult<Self> {
        let t = bearer(&p.headers).ok_or_else(ApiError::unauthorized)?;
        if let Some(st) = &app.cfg.service_token {
            if crypto::ct_eq(st.as_bytes(), t.as_bytes()) {
                return Ok(Admin {
                    actor: "service".into(),
                });
            }
        }
        let c = app.verify_access(t).ok_or_else(ApiError::unauthorized)?;
        if !c.roles.iter().any(|r| r == "admin") {
            return Err(ApiError::forbidden());
        }
        let mfa = c
            .amr
            .iter()
            .any(|m| matches!(m.as_str(), "otp" | "mfa" | "hwk"));
        if app.cfg.admin_require_mfa && !mfa {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "mfa_required",
                "admin endpoints require a multi-factor login",
            ));
        }
        Ok(Admin { actor: c.sub })
    }
}

// ---------------------------------------------------------------- router

pub fn router(app: Arc<App>) -> Router {
    let origins: Vec<HeaderValue> = app
        .cfg
        .allowed_origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            AUTHORIZATION,
            axum::http::HeaderName::from_static(CSRF_HEADER),
        ]);
    let limited = Router::new()
        .route("/v1/register", post(register))
        .route("/v1/verify-email", post(verify_email))
        .route("/v1/verify-email/resend", post(resend_verification))
        .route("/v1/password/forgot", post(forgot_password))
        .route("/v1/password/reset", post(reset_password))
        .route("/v1/login", post(login))
        .route("/v1/login/2fa", post(login_2fa))
        .route("/v1/token/refresh", post(refresh))
        .route("/v1/token/revoke", post(revoke))
        .route("/v1/passkeys/login/start", post(passkey_login_start))
        .route("/v1/passkeys/login/finish", post(passkey_login_finish))
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            rate_limit,
        ));
    // Authenticated routes that change security state or are worth guessing
    // (TOTP codes, admin): same per-IP limiter as the login routes.
    let sensitive = Router::new()
        .route("/v1/sessions/revoke-all", post(revoke_all))
        .route("/v1/2fa/totp/enroll", post(totp_enroll))
        .route("/v1/2fa/totp/confirm", post(totp_confirm))
        .route("/v1/2fa/totp/disable", post(totp_disable))
        .route("/v1/passkeys/register/start", post(passkey_register_start))
        .route(
            "/v1/passkeys/register/finish",
            post(passkey_register_finish),
        )
        .route("/v1/admin/users", get(admin_user))
        .route("/v1/admin/users/roles", post(admin_set_roles))
        .route("/v1/admin/accounts/link", post(admin_link))
        .route("/v1/admin/accounts/unlink", post(admin_unlink))
        .route("/v1/admin/audit", get(admin_audit))
        .route("/v1/admin/keys/rotate", post(admin_rotate_keys))
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            rate_limit,
        ));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/.well-known/jwks.json", get(jwks))
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/v1/me", get(me))
        .route("/v1/accounts", get(my_accounts))
        .merge(sensitive)
        .merge(limited)
        .layer(cors)
        .with_state(app)
}

/// Keyed limiter entries above which idle keys are pruned (bounded memory
/// against spoofed / many client addresses).
const LIMITER_PRUNE_AT: usize = 10_000;

async fn rate_limit(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let ip = client_ip(&app, req.headers(), req.extensions()).unwrap_or_else(|| "unknown".into());
    if app.ip_limiter.len() > LIMITER_PRUNE_AT {
        app.ip_limiter.retain_recent();
        app.ip_limiter.shrink_to_fit();
    }
    if app.ip_limiter.check_key(&ip).is_err() {
        app.audit(None, "rate_limited", Some(&ip), req.uri().path())
            .await;
        return ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "too many requests, slow down",
        )
        .into_response();
    }
    next.run(req).await
}

// ---------------------------------------------------------------- discovery

async fn jwks(State(app): State<Arc<App>>) -> impl IntoResponse {
    (
        [(axum::http::header::CACHE_CONTROL, "public, max-age=300")],
        Json(app.keys.jwks()),
    )
}

async fn discovery(State(app): State<Arc<App>>) -> Json<Value> {
    let i = &app.cfg.issuer;
    Json(json!({
        "issuer": i,
        "jwks_uri": format!("{i}/.well-known/jwks.json"),
        "token_endpoint": format!("{i}/v1/token/refresh"),
        "revocation_endpoint": format!("{i}/v1/token/revoke"),
        "userinfo_endpoint": format!("{i}/v1/me"),
        "response_types_supported": ["token"],
        "grant_types_supported": ["password", "refresh_token"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "claims_supported": ["iss", "aud", "sub", "exp", "iat", "jti", "sid", "email", "accounts", "roles", "amr"],
        "fxvps_roles": ROLES,
    }))
}

// ---------------------------------------------------------------- sessions

/// How the refresh token is delivered.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionMode {
    /// Response body (desktop / mobile).
    #[default]
    Bearer,
    /// `HttpOnly; Secure; SameSite=Strict` cookie scoped to `/v1/token` (web).
    Cookie,
}

#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    refresh_expires_in: u64,
    accounts: Vec<String>,
    roles: Vec<String>,
}

fn cookie(app: &App, value: &str, max_age: u64) -> HeaderValue {
    let secure = if app.cfg.cookie_secure {
        "; Secure"
    } else {
        ""
    };
    HeaderValue::from_str(&format!(
        "{REFRESH_COOKIE}={value}; Path={COOKIE_PATH}; Max-Age={max_age}; HttpOnly; SameSite=Strict{secure}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static(""))
}

async fn access_token(
    app: &App,
    user: &User,
    sid: &str,
    amr: &[String],
) -> ApiResult<(String, Vec<String>)> {
    let accounts = app.store.accounts(&user.id).await?;
    let iat = now() as u64;
    let claims = AccessClaims {
        iss: app.cfg.issuer.clone(),
        aud: app.cfg.audience.clone(),
        sub: user.id.clone(),
        exp: iat + app.cfg.access_ttl.as_secs(),
        iat,
        jti: random_id(),
        sid: sid.into(),
        email: user.email.clone(),
        accounts: accounts.clone(),
        roles: user.roles.clone(),
        amr: amr.to_vec(),
    };
    let t = app.keys.sign(&claims).map_err(|e| {
        tracing::error!(error = %e, "sign");
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "signing failed",
        )
    })?;
    Ok((t, accounts))
}

fn token_response(
    app: &App,
    user: &User,
    access: String,
    accounts: Vec<String>,
    refresh: String,
    mode: SessionMode,
) -> Response {
    let rt = app.cfg.refresh_ttl.as_secs();
    let body = TokenResponse {
        access_token: access,
        token_type: "Bearer",
        expires_in: app.cfg.access_ttl.as_secs(),
        refresh_token: (mode == SessionMode::Bearer).then(|| refresh.clone()),
        refresh_expires_in: rt,
        accounts,
        roles: user.roles.clone(),
    };
    let mut r = Json(body).into_response();
    if mode == SessionMode::Cookie {
        r.headers_mut()
            .insert(SET_COOKIE, cookie(app, &refresh, rt));
    }
    r.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    r
}

/// Starts a new session (token family) after a successful login.
async fn new_session(
    app: &App,
    user: &User,
    amr: Vec<String>,
    mode: SessionMode,
    ip: Option<&str>,
) -> ApiResult<Response> {
    let refresh = random_token();
    let t = now();
    let family = random_id();
    app.store
        .insert_refresh(&RefreshToken {
            hash: sha256_hex(&refresh),
            user_id: user.id.clone(),
            family_id: family.clone(),
            amr: amr.clone(),
            created_at: t,
            expires_at: t + app.cfg.refresh_ttl.as_secs() as i64,
            revoked: false,
            rotated: false,
        })
        .await?;
    let (access, accounts) = access_token(app, user, &family, &amr).await?;
    app.audit(
        Some(&user.id),
        "login_success",
        ip,
        &format!("amr={}", amr.join(",")),
    )
    .await;
    Ok(token_response(app, user, access, accounts, refresh, mode))
}

fn read_cookie(h: &HeaderMap) -> Option<String> {
    h.get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == REFRESH_COOKIE)
        .map(|(_, v)| v.to_string())
}

/// Refresh token from the body (bearer mode) or the cookie (web). Cookie mode
/// requires the CSRF header and, when present, an allowed `Origin`.
fn presented_refresh(
    app: &App,
    h: &HeaderMap,
    body: Option<String>,
) -> ApiResult<(String, SessionMode)> {
    if let Some(t) = body.filter(|t| !t.is_empty()) {
        return Ok((t, SessionMode::Bearer));
    }
    let t = read_cookie(h).ok_or_else(ApiError::unauthorized)?;
    if h.get(CSRF_HEADER).is_none() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "csrf",
            "missing CSRF header",
        ));
    }
    if let Some(o) = h.get(ORIGIN).and_then(|v| v.to_str().ok()) {
        if !app.cfg.allowed_origins.iter().any(|a| a == o) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "csrf",
                "origin not allowed",
            ));
        }
    }
    Ok((t, SessionMode::Cookie))
}

#[derive(Deserialize, Default)]
struct RefreshReq {
    #[serde(default)]
    refresh_token: Option<String>,
}

async fn refresh(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    body: Option<Json<RefreshReq>>,
) -> ApiResult<Response> {
    let body = body.map(|b| b.0).unwrap_or_default();
    let (presented, mode) = presented_refresh(&app, &headers, body.refresh_token)?;
    let old_hash = sha256_hex(&presented);
    let Some(old) = app.store.get_refresh(&old_hash).await? else {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_grant",
            "invalid refresh token",
        ));
    };
    let next = random_token();
    let t = now();
    let new = RefreshToken {
        hash: sha256_hex(&next),
        user_id: old.user_id.clone(),
        family_id: old.family_id.clone(),
        amr: old.amr.clone(),
        created_at: t,
        expires_at: t + app.cfg.refresh_ttl.as_secs() as i64,
        revoked: false,
        rotated: false,
    };
    match app.store.rotate_refresh(&old_hash, &new, t).await? {
        Rotation::Rotated(n) => {
            let Some(user) = app.store.user_by_id(&n.user_id).await? else {
                return Err(ApiError::unauthorized());
            };
            let (access, accounts) = access_token(&app, &user, &n.family_id, &n.amr).await?;
            app.audit(Some(&user.id), "token_refreshed", ip.as_deref(), "")
                .await;
            Ok(token_response(&app, &user, access, accounts, next, mode))
        }
        Rotation::Reused { user_id, family_id } => {
            app.audit(
                Some(&user_id),
                "refresh_token_reuse",
                ip.as_deref(),
                &format!("family {family_id} revoked"),
            )
            .await;
            Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "invalid_grant",
                "refresh token reused; session revoked",
            ))
        }
        Rotation::Invalid => Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_grant",
            "refresh token expired",
        )),
    }
}

async fn revoke(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    body: Option<Json<RefreshReq>>,
) -> ApiResult<Response> {
    let body = body.map(|b| b.0).unwrap_or_default();
    let (presented, mode) = presented_refresh(&app, &headers, body.refresh_token)?;
    if let Some(t) = app.store.get_refresh(&sha256_hex(&presented)).await? {
        app.store.revoke_family(&t.family_id).await?;
        app.audit(Some(&t.user_id), "logout", ip.as_deref(), "")
            .await;
    }
    let mut r = StatusCode::NO_CONTENT.into_response();
    if mode == SessionMode::Cookie {
        r.headers_mut().insert(SET_COOKIE, cookie(&app, "", 0));
    }
    Ok(r)
}

async fn revoke_all(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    Authed(c): Authed,
) -> ApiResult<Json<Value>> {
    let n = app.store.revoke_user_sessions(&c.sub).await?;
    app.audit(
        Some(&c.sub),
        "sessions_revoked",
        ip.as_deref(),
        &format!("{n} sessions"),
    )
    .await;
    Ok(Json(json!({ "revoked": n })))
}

// ---------------------------------------------------------------- registration

fn normalize_email(e: &str) -> ApiResult<String> {
    let e = e.trim().to_lowercase();
    let ok = e.len() <= 254
        && e.split_once('@').is_some_and(|(l, d)| {
            !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.')
        })
        && !e.contains(char::is_whitespace);
    if ok {
        Ok(e)
    } else {
        Err(ApiError::bad("invalid_email", "invalid email address"))
    }
}

fn check_password(p: &str) -> ApiResult<()> {
    if p.chars().count() < 10 || p.len() > 256 {
        return Err(ApiError::bad(
            "weak_password",
            "password must be 10 to 256 characters",
        ));
    }
    Ok(())
}

async fn send_link(app: &App, user: &User, kind: TokenKind) -> ApiResult<()> {
    let token = random_token();
    let ttl = match kind {
        TokenKind::VerifyEmail => app.cfg.verify_ttl,
        _ => app.cfg.reset_ttl,
    };
    app.store.delete_tokens(&user.id, kind).await?;
    app.store
        .put_token(&OneTimeToken {
            hash: sha256_hex(&token),
            kind,
            user_id: user.id.clone(),
            expires_at: now() + ttl.as_secs() as i64,
            data: None,
            attempts: 0,
        })
        .await?;
    let (param, subject, text) = match kind {
        TokenKind::VerifyEmail => (
            "verify_email",
            "Verify your fxvps email",
            "Confirm your email address",
        ),
        _ => (
            "reset_password",
            "Reset your fxvps password",
            "Reset your password",
        ),
    };
    let link = format!("{}/?{param}={token}", app.cfg.public_url);
    let mail = Mail {
        to: user.email.clone(),
        subject: subject.into(),
        body: format!(
            "{text}: {link}\nThis link expires in {} minutes.",
            ttl.as_secs() / 60
        ),
        link: Some(link),
    };
    if let Err(e) = app.mailer.send(mail).await {
        tracing::error!(error = %e, "mail send failed");
    }
    Ok(())
}

#[derive(Deserialize)]
struct RegisterReq {
    email: String,
    password: String,
}

/// Always `202` (does not reveal whether the email is registered).
async fn register(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    Json(r): Json<RegisterReq>,
) -> ApiResult<Response> {
    let email = normalize_email(&r.email)?;
    check_password(&r.password)?;
    let mut roles = vec!["client".to_string()];
    if app.cfg.bootstrap_admins.contains(&email) {
        roles.push("admin".into());
    }
    let user = User {
        id: random_id(),
        email: email.clone(),
        password_hash: app.passwords.hash(&r.password),
        email_verified: false,
        roles,
        totp_secret: None,
        totp_enabled: false,
        totp_last_step: 0,
        recovery_codes: Vec::new(),
        failed_logins: 0,
        locked_until: 0,
        created_at: now(),
    };
    match app.store.create_user(&user).await {
        Ok(()) => {
            app.audit(Some(&user.id), "register", ip.as_deref(), "")
                .await;
            send_link(&app, &user, TokenKind::VerifyEmail).await?;
        }
        Err(StoreError::Conflict) => {
            app.audit(None, "register_existing_email", ip.as_deref(), "")
                .await;
            if let Some(existing) = app.store.user_by_email(&email).await? {
                let _ = app
                    .mailer
                    .send(Mail {
                        to: existing.email,
                        subject: "fxvps sign-up attempt".into(),
                        body: "Someone tried to register with your email. If it was you, sign in or reset your password.".into(),
                        link: None,
                    })
                    .await;
            }
        }
        Err(e) => return Err(e.into()),
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "status": "verification_sent" })),
    )
        .into_response())
}

#[derive(Deserialize)]
struct TokenReq {
    token: String,
}

async fn verify_email(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    Json(r): Json<TokenReq>,
) -> ApiResult<Json<Value>> {
    let t = app
        .store
        .take_token(&sha256_hex(&r.token), TokenKind::VerifyEmail)
        .await?
        .filter(|t| t.expires_at > now())
        .ok_or_else(|| ApiError::bad("invalid_token", "invalid or expired link"))?;
    let mut u = app
        .store
        .user_by_id(&t.user_id)
        .await?
        .ok_or(StoreError::NotFound)?;
    u.email_verified = true;
    app.store.update_user(&u).await?;
    app.audit(Some(&u.id), "email_verified", ip.as_deref(), "")
        .await;
    Ok(Json(json!({ "status": "verified" })))
}

#[derive(Deserialize)]
struct EmailReq {
    email: String,
}

async fn resend_verification(
    State(app): State<Arc<App>>,
    Json(r): Json<EmailReq>,
) -> ApiResult<Response> {
    if let Ok(e) = normalize_email(&r.email) {
        if let Some(u) = app
            .store
            .user_by_email(&e)
            .await?
            .filter(|u| !u.email_verified)
        {
            send_link(&app, &u, TokenKind::VerifyEmail).await?;
        }
    }
    Ok(StatusCode::ACCEPTED.into_response())
}

async fn forgot_password(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    Json(r): Json<EmailReq>,
) -> ApiResult<Response> {
    if let Ok(e) = normalize_email(&r.email) {
        if let Some(u) = app.store.user_by_email(&e).await? {
            send_link(&app, &u, TokenKind::PasswordReset).await?;
            app.audit(Some(&u.id), "password_reset_requested", ip.as_deref(), "")
                .await;
        }
    }
    Ok(StatusCode::ACCEPTED.into_response())
}

#[derive(Deserialize)]
struct ResetReq {
    token: String,
    password: String,
}

/// Sets a new password, clears lockout and revokes every session.
async fn reset_password(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    Json(r): Json<ResetReq>,
) -> ApiResult<Json<Value>> {
    check_password(&r.password)?;
    let t = app
        .store
        .take_token(&sha256_hex(&r.token), TokenKind::PasswordReset)
        .await?
        .filter(|t| t.expires_at > now())
        .ok_or_else(|| ApiError::bad("invalid_token", "invalid or expired link"))?;
    let mut u = app
        .store
        .user_by_id(&t.user_id)
        .await?
        .ok_or(StoreError::NotFound)?;
    u.password_hash = app.passwords.hash(&r.password);
    u.failed_logins = 0;
    u.locked_until = 0;
    // Receiving the link proves control of the mailbox.
    u.email_verified = true;
    app.store.update_user(&u).await?;
    let n = app.store.revoke_user_sessions(&u.id).await?;
    app.audit(
        Some(&u.id),
        "password_reset",
        ip.as_deref(),
        &format!("{n} sessions revoked"),
    )
    .await;
    Ok(Json(json!({ "status": "password_changed" })))
}

// ---------------------------------------------------------------- login

#[derive(Deserialize)]
struct LoginReq {
    email: String,
    password: String,
    #[serde(default)]
    session: SessionMode,
}

fn locked(u: &User, t: i64) -> Option<ApiError> {
    (u.locked_until > t).then(|| {
        ApiError::new(
            StatusCode::LOCKED,
            "account_locked",
            format!(
                "too many failed attempts; retry in {} s",
                u.locked_until - t
            ),
        )
    })
}

/// Counts a failed attempt; locks the account at the threshold.
async fn register_failure(app: &App, u: &mut User, ip: Option<&str>, what: &str) -> ApiResult<()> {
    u.failed_logins += 1;
    if u.failed_logins >= app.cfg.max_failed_logins {
        u.locked_until = now() + app.cfg.lockout.as_secs() as i64;
        u.failed_logins = 0;
        app.store.update_user(u).await?;
        app.audit(Some(&u.id), "account_locked", ip, what).await;
    } else {
        app.store.update_user(u).await?;
        app.audit(Some(&u.id), "login_failed", ip, what).await;
    }
    Ok(())
}

async fn login(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    Json(r): Json<LoginReq>,
) -> ApiResult<Response> {
    let ip = ip.as_deref();
    let email = r.email.trim().to_lowercase();
    let Some(mut u) = app.store.user_by_email(&email).await? else {
        app.passwords.burn(&r.password);
        app.audit(None, "login_failed", ip, "unknown email").await;
        return Err(ApiError::invalid_credentials());
    };
    let t = now();
    if let Some(e) = locked(&u, t) {
        app.audit(Some(&u.id), "login_locked", ip, "").await;
        return Err(e);
    }
    if !app.passwords.verify(&r.password, &u.password_hash) {
        register_failure(&app, &mut u, ip, "password").await?;
        return Err(ApiError::invalid_credentials());
    }
    if !u.email_verified {
        app.audit(Some(&u.id), "login_unverified", ip, "").await;
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "email_not_verified",
            "verify your email first",
        ));
    }
    if u.totp_enabled {
        let token = random_token();
        app.store
            .put_token(&OneTimeToken {
                hash: sha256_hex(&token),
                kind: TokenKind::Mfa,
                user_id: u.id.clone(),
                expires_at: t + app.cfg.mfa_ttl.as_secs() as i64,
                data: None,
                attempts: 0,
            })
            .await?;
        app.audit(Some(&u.id), "login_mfa_required", ip, "").await;
        return Ok(Json(json!({
            "mfa_required": true,
            "mfa_token": token,
            "methods": ["totp", "recovery_code"],
        }))
        .into_response());
    }
    // The failure counter is only reset after full authentication, so a known
    // password does not grant unlimited second-factor guesses.
    if u.failed_logins != 0 {
        u.failed_logins = 0;
        app.store.update_user(&u).await?;
    }
    new_session(&app, &u, vec!["pwd".into()], r.session, ip).await
}

#[derive(Deserialize)]
struct Login2faReq {
    mfa_token: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    recovery_code: Option<String>,
    #[serde(default)]
    session: SessionMode,
}

async fn login_2fa(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    Json(r): Json<Login2faReq>,
) -> ApiResult<Response> {
    let ip = ip.as_deref();
    let hash = sha256_hex(&r.mfa_token);
    let t = now();
    let tok = app
        .store
        .get_token(&hash, TokenKind::Mfa)
        .await?
        .filter(|x| x.expires_at > t)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "invalid_mfa_token",
                "sign in again",
            )
        })?;
    let mut u = app
        .store
        .user_by_id(&tok.user_id)
        .await?
        .ok_or(StoreError::NotFound)?;
    if let Some(e) = locked(&u, t) {
        app.store.take_token(&hash, TokenKind::Mfa).await?;
        return Err(e);
    }
    let mut amr = vec!["pwd".to_string(), "mfa".to_string()];
    let ok = if let Some(code) = r.code.as_deref() {
        amr.push("otp".into());
        match u
            .totp_secret
            .as_deref()
            .and_then(|s| crypto::totp(s, &u.email))
        {
            Some(totp) => match crypto::totp_check(&totp, code, t as u64, u.totp_last_step) {
                Some(step) => {
                    u.totp_last_step = step;
                    true
                }
                None => false,
            },
            None => false,
        }
    } else if let Some(rc) = r.recovery_code.as_deref() {
        let h = sha256_hex(&crypto::normalize_recovery(rc));
        match u
            .recovery_codes
            .iter()
            .position(|c| crypto::ct_eq(c.as_bytes(), h.as_bytes()))
        {
            Some(i) => {
                u.recovery_codes.remove(i);
                amr.push("rc".into());
                true
            }
            None => false,
        }
    } else {
        return Err(ApiError::bad(
            "missing_code",
            "code or recovery_code required",
        ));
    };
    if !ok {
        let n = app.store.bump_token_attempts(&hash).await?;
        if n >= MFA_MAX_ATTEMPTS {
            app.store.take_token(&hash, TokenKind::Mfa).await?;
        }
        register_failure(&app, &mut u, ip, "second factor").await?;
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_code",
            "invalid code",
        ));
    }
    if app.store.take_token(&hash, TokenKind::Mfa).await?.is_none() {
        // Raced with another request using the same MFA token.
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_mfa_token",
            "sign in again",
        ));
    }
    u.failed_logins = 0;
    app.store.update_user(&u).await?;
    if amr.iter().any(|a| a == "rc") {
        app.audit(
            Some(&u.id),
            "recovery_code_used",
            ip,
            &format!("{} left", u.recovery_codes.len()),
        )
        .await;
    }
    new_session(&app, &u, amr, r.session, ip).await
}

// ---------------------------------------------------------------- account

async fn me(State(app): State<Arc<App>>, Authed(c): Authed) -> ApiResult<Json<Value>> {
    let u = app
        .store
        .user_by_id(&c.sub)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let accounts = app.store.accounts(&u.id).await?;
    let passkeys: Vec<Value> = app
        .store
        .passkeys(&u.id)
        .await?
        .into_iter()
        .map(|p| json!({ "id": p.cred_id, "name": p.name, "created_at": p.created_at }))
        .collect();
    Ok(Json(json!({
        "id": u.id,
        "email": u.email,
        "email_verified": u.email_verified,
        "roles": u.roles,
        "accounts": accounts,
        "totp_enabled": u.totp_enabled,
        "recovery_codes_left": u.recovery_codes.len(),
        "passkeys": passkeys,
        "amr": c.amr,
    })))
}

async fn my_accounts(State(app): State<Arc<App>>, Authed(c): Authed) -> ApiResult<Json<Value>> {
    Ok(Json(
        json!({ "accounts": app.store.accounts(&c.sub).await? }),
    ))
}

// ---------------------------------------------------------------- TOTP

async fn totp_enroll(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    Authed(c): Authed,
) -> ApiResult<Json<Value>> {
    let mut u = app
        .store
        .user_by_id(&c.sub)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if u.totp_enabled {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "totp_enabled",
            "TOTP already enabled",
        ));
    }
    let secret = crypto::new_totp_secret();
    let url = crypto::totp(&secret, &u.email)
        .map(|t| t.get_url())
        .unwrap_or_default();
    u.totp_secret = Some(secret.clone());
    app.store.update_user(&u).await?;
    app.audit(Some(&u.id), "totp_enroll_started", ip.as_deref(), "")
        .await;
    Ok(Json(json!({ "secret": secret, "otpauth_url": url })))
}

#[derive(Deserialize)]
struct CodeReq {
    code: String,
}

/// Enables TOTP and returns 10 one-time recovery codes (shown once).
async fn totp_confirm(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    Authed(c): Authed,
    Json(r): Json<CodeReq>,
) -> ApiResult<Json<Value>> {
    let mut u = app
        .store
        .user_by_id(&c.sub)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if u.totp_enabled {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "totp_enabled",
            "TOTP already enabled",
        ));
    }
    let totp = u
        .totp_secret
        .as_deref()
        .and_then(|s| crypto::totp(s, &u.email))
        .ok_or_else(|| ApiError::bad("not_enrolling", "start enrollment first"))?;
    if let Some(e) = locked(&u, now()) {
        return Err(e);
    }
    let Some(step) = crypto::totp_check(&totp, &r.code, now() as u64, u.totp_last_step) else {
        register_failure(&app, &mut u, ip.as_deref(), "totp confirm").await?;
        return Err(ApiError::bad("invalid_code", "invalid code"));
    };
    let codes: Vec<String> = (0..10).map(|_| crypto::recovery_code()).collect();
    u.recovery_codes = codes.iter().map(|c| sha256_hex(c)).collect();
    u.totp_enabled = true;
    u.totp_last_step = step;
    app.store.update_user(&u).await?;
    app.audit(Some(&u.id), "totp_enabled", ip.as_deref(), "")
        .await;
    Ok(Json(json!({ "recovery_codes": codes })))
}

async fn totp_disable(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    Authed(c): Authed,
    Json(r): Json<CodeReq>,
) -> ApiResult<Json<Value>> {
    let mut u = app
        .store
        .user_by_id(&c.sub)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let totp = u
        .totp_secret
        .as_deref()
        .filter(|_| u.totp_enabled)
        .and_then(|s| crypto::totp(s, &u.email))
        .ok_or_else(|| ApiError::bad("totp_disabled", "TOTP is not enabled"))?;
    if let Some(e) = locked(&u, now()) {
        return Err(e);
    }
    if crypto::totp_check(&totp, &r.code, now() as u64, u.totp_last_step).is_none() {
        register_failure(&app, &mut u, ip.as_deref(), "totp disable").await?;
        return Err(ApiError::bad("invalid_code", "invalid code"));
    }
    u.failed_logins = 0;
    u.totp_enabled = false;
    u.totp_secret = None;
    u.recovery_codes.clear();
    app.store.update_user(&u).await?;
    app.audit(Some(&u.id), "totp_disabled", ip.as_deref(), "")
        .await;
    Ok(Json(json!({ "status": "disabled" })))
}

// ---------------------------------------------------------------- passkeys

fn wa_err(e: impl std::fmt::Display) -> ApiError {
    ApiError::bad("webauthn", e.to_string())
}

fn ser<T: Serialize>(v: &T) -> ApiResult<String> {
    serde_json::to_string(v)
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))
}

fn de<T: serde::de::DeserializeOwned>(s: &str) -> ApiResult<T> {
    serde_json::from_str(s)
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))
}

async fn user_passkeys(app: &App, user_id: &str) -> ApiResult<Vec<(PasskeyRecord, Passkey)>> {
    let mut out = Vec::new();
    for r in app.store.passkeys(user_id).await? {
        let p: Passkey = de(&r.data)?;
        out.push((r, p));
    }
    Ok(out)
}

async fn passkey_register_start(
    State(app): State<Arc<App>>,
    Authed(c): Authed,
) -> ApiResult<Json<Value>> {
    let u = app
        .store
        .user_by_id(&c.sub)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let uid = Uuid::parse_str(&u.id).map_err(wa_err)?;
    let exclude = user_passkeys(&app, &u.id)
        .await?
        .into_iter()
        .map(|(_, p)| p.cred_id().clone())
        .collect::<Vec<_>>();
    let (options, state) = app
        .webauthn
        .start_passkey_registration(uid, &u.email, &u.email, Some(exclude))
        .map_err(wa_err)?;
    let id = random_token();
    app.store
        .put_token(&OneTimeToken {
            hash: sha256_hex(&id),
            kind: TokenKind::PasskeyRegistration,
            user_id: u.id,
            expires_at: now() + 300,
            data: Some(ser(&state)?),
            attempts: 0,
        })
        .await?;
    Ok(Json(json!({ "registration_id": id, "options": options })))
}

#[derive(Deserialize)]
struct RegFinishReq {
    registration_id: String,
    credential: RegisterPublicKeyCredential,
    #[serde(default)]
    name: Option<String>,
}

async fn passkey_register_finish(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    Authed(c): Authed,
    Json(r): Json<RegFinishReq>,
) -> ApiResult<Json<Value>> {
    let t = app
        .store
        .take_token(
            &sha256_hex(&r.registration_id),
            TokenKind::PasskeyRegistration,
        )
        .await?
        .filter(|t| t.expires_at > now() && t.user_id == c.sub)
        .ok_or_else(|| ApiError::bad("invalid_registration", "registration expired"))?;
    let state: PasskeyRegistration = de(t.data.as_deref().unwrap_or("null"))?;
    let pk = app
        .webauthn
        .finish_passkey_registration(&r.credential, &state)
        .map_err(wa_err)?;
    let cred_id = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        pk.cred_id(),
    );
    let name = r
        .name
        .unwrap_or_else(|| "passkey".into())
        .chars()
        .take(64)
        .collect();
    app.store
        .add_passkey(&PasskeyRecord {
            user_id: c.sub.clone(),
            cred_id: cred_id.clone(),
            name,
            data: ser(&pk)?,
            created_at: now(),
        })
        .await?;
    app.audit(Some(&c.sub), "passkey_registered", ip.as_deref(), &cred_id)
        .await;
    Ok(Json(json!({ "id": cred_id })))
}

#[derive(Deserialize)]
struct PkLoginStartReq {
    email: String,
}

async fn passkey_login_start(
    State(app): State<Arc<App>>,
    Json(r): Json<PkLoginStartReq>,
) -> ApiResult<Json<Value>> {
    let email = r.email.trim().to_lowercase();
    let u = app.store.user_by_email(&email).await?;
    let keys = match &u {
        Some(u) => user_passkeys(&app, &u.id).await?,
        None => Vec::new(),
    };
    let Some(u) = u.filter(|_| !keys.is_empty()) else {
        return Err(ApiError::bad(
            "no_passkeys",
            "no passkey registered for this account",
        ));
    };
    let creds: Vec<Passkey> = keys.into_iter().map(|(_, p)| p).collect();
    let (options, state) = app
        .webauthn
        .start_passkey_authentication(&creds)
        .map_err(wa_err)?;
    let id = random_token();
    app.store
        .put_token(&OneTimeToken {
            hash: sha256_hex(&id),
            kind: TokenKind::PasskeyAuthentication,
            user_id: u.id,
            expires_at: now() + 300,
            data: Some(ser::<PasskeyAuthentication>(&state)?),
            attempts: 0,
        })
        .await?;
    Ok(Json(json!({ "authentication_id": id, "options": options })))
}

#[derive(Deserialize)]
struct PkLoginFinishReq {
    authentication_id: String,
    credential: PublicKeyCredential,
    #[serde(default)]
    session: SessionMode,
}

async fn passkey_login_finish(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    Json(r): Json<PkLoginFinishReq>,
) -> ApiResult<Response> {
    let ip = ip.as_deref();
    let t = app
        .store
        .take_token(
            &sha256_hex(&r.authentication_id),
            TokenKind::PasskeyAuthentication,
        )
        .await?
        .filter(|t| t.expires_at > now())
        .ok_or_else(|| ApiError::bad("invalid_authentication", "authentication expired"))?;
    let mut u = app
        .store
        .user_by_id(&t.user_id)
        .await?
        .ok_or(StoreError::NotFound)?;
    if let Some(e) = locked(&u, now()) {
        return Err(e);
    }
    let state: PasskeyAuthentication = de(t.data.as_deref().unwrap_or("null"))?;
    let res = match app
        .webauthn
        .finish_passkey_authentication(&r.credential, &state)
    {
        Ok(r) => r,
        Err(e) => {
            register_failure(&app, &mut u, ip, "passkey").await?;
            return Err(wa_err(e));
        }
    };
    for (rec, mut pk) in user_passkeys(&app, &u.id).await? {
        if pk.update_credential(&res) == Some(true) {
            app.store.update_passkey(&rec.cred_id, &ser(&pk)?).await?;
        }
    }
    if !u.email_verified {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "email_not_verified",
            "verify your email first",
        ));
    }
    new_session(&app, &u, vec!["hwk".into(), "mfa".into()], r.session, ip).await
}

// ---------------------------------------------------------------- admin

#[derive(Deserialize)]
struct UserQuery {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

async fn find_user(app: &App, id: Option<&str>, email: Option<&str>) -> ApiResult<User> {
    let u = match (id, email) {
        (Some(id), _) => app.store.user_by_id(id).await?,
        (None, Some(e)) => app.store.user_by_email(&e.trim().to_lowercase()).await?,
        _ => return Err(ApiError::bad("missing_user", "user_id or email required")),
    };
    u.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "user_not_found", "no such user"))
}

async fn admin_user(
    State(app): State<Arc<App>>,
    _a: Admin,
    Query(q): Query<UserQuery>,
) -> ApiResult<Json<Value>> {
    let u = find_user(&app, q.id.as_deref(), q.email.as_deref()).await?;
    Ok(Json(json!({
        "id": u.id,
        "email": u.email,
        "email_verified": u.email_verified,
        "roles": u.roles,
        "totp_enabled": u.totp_enabled,
        "locked_until": u.locked_until,
        "accounts": app.store.accounts(&u.id).await?,
        "created_at": u.created_at,
    })))
}

#[derive(Deserialize)]
struct RolesReq {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    email: Option<String>,
    roles: Vec<String>,
}

async fn admin_set_roles(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    a: Admin,
    Json(r): Json<RolesReq>,
) -> ApiResult<Json<Value>> {
    if let Some(bad) = r.roles.iter().find(|x| !ROLES.contains(&x.as_str())) {
        return Err(ApiError::bad("invalid_role", format!("unknown role {bad}")));
    }
    let mut u = find_user(&app, r.user_id.as_deref(), r.email.as_deref()).await?;
    let mut roles = r.roles.clone();
    roles.sort();
    roles.dedup();
    u.roles = roles;
    app.store.update_user(&u).await?;
    app.audit(
        Some(&u.id),
        "roles_changed",
        ip.as_deref(),
        &format!("by {}: {}", a.actor, u.roles.join(",")),
    )
    .await;
    Ok(Json(json!({ "id": u.id, "roles": u.roles })))
}

#[derive(Deserialize)]
struct LinkReq {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    email: Option<String>,
    account_id: String,
}

/// Links a trading account to a user (idempotent). Called by core-engine / back
/// office after opening an account. `409` if another user owns the account.
async fn admin_link(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    a: Admin,
    Json(r): Json<LinkReq>,
) -> ApiResult<Json<Value>> {
    let acct = r.account_id.trim();
    if acct.is_empty() || acct.len() > 64 {
        return Err(ApiError::bad(
            "invalid_account",
            "account_id must be 1..64 characters",
        ));
    }
    let u = find_user(&app, r.user_id.as_deref(), r.email.as_deref()).await?;
    app.store.link_account(&u.id, acct).await?;
    app.audit(
        Some(&u.id),
        "account_linked",
        ip.as_deref(),
        &format!("{acct} by {}", a.actor),
    )
    .await;
    Ok(Json(
        json!({ "user_id": u.id, "accounts": app.store.accounts(&u.id).await? }),
    ))
}

async fn admin_unlink(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    a: Admin,
    Json(r): Json<LinkReq>,
) -> ApiResult<Json<Value>> {
    let u = find_user(&app, r.user_id.as_deref(), r.email.as_deref()).await?;
    let removed = app.store.unlink_account(&u.id, r.account_id.trim()).await?;
    if removed {
        app.audit(
            Some(&u.id),
            "account_unlinked",
            ip.as_deref(),
            &format!("{} by {}", r.account_id, a.actor),
        )
        .await;
    }
    Ok(Json(
        json!({ "user_id": u.id, "removed": removed, "accounts": app.store.accounts(&u.id).await? }),
    ))
}

#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

async fn admin_audit(
    State(app): State<Arc<App>>,
    _a: Admin,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Value>> {
    let ev = app
        .store
        .audit_events(q.user_id.as_deref(), q.limit.unwrap_or(100).clamp(1, 1000))
        .await?;
    Ok(Json(json!({ "events": ev })))
}

/// Generates a new signing key in memory; the old key stays in the JWKS.
/// (With file-configured keys, rotate by deploying a new `IDENTITY_SIGNING_KEY_FILE`
/// and listing the old one in `IDENTITY_RETIRED_KEY_FILES`.)
async fn admin_rotate_keys(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    a: Admin,
) -> ApiResult<Json<Value>> {
    let k = tokio::task::spawn_blocking(crate::keys::generate_rsa)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let kid = app
        .keys
        .rotate(&k)
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    app.audit(
        None,
        "signing_key_rotated",
        ip.as_deref(),
        &format!("kid {kid} by {}", a.actor),
    )
    .await;
    Ok(Json(json!({ "kid": kid })))
}

#[cfg(test)]
mod xff_tests {
    use super::forwarded_client_ip as f;

    #[test]
    fn rightmost_trusted_hop_wins() {
        // client spoofs "1.1.1.1"; the trusted proxy appended the real peer.
        assert_eq!(f("1.1.1.1, 203.0.113.7", 1).as_deref(), Some("203.0.113.7"));
        assert_eq!(f("203.0.113.7", 1).as_deref(), Some("203.0.113.7"));
        // two trusted proxies (CDN + ingress)
        assert_eq!(
            f("6.6.6.6, 203.0.113.7, 10.0.0.2", 2).as_deref(),
            Some("203.0.113.7")
        );
        // shorter than the configured hops / garbage: no forwarded IP
        assert_eq!(f("203.0.113.7", 2), None);
        assert_eq!(f("1.1.1.1, not-an-ip", 1), None);
        assert_eq!(f("::1", 1).as_deref(), Some("::1"));
    }
}
