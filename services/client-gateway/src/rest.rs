//! User REST API (`/api/v1`): accounts, positions, orders, deals, symbols and
//! quotes over plain HTTP, next to the WebSocket protocol.
//!
//! Authentication is `Authorization: Bearer <access token>`: the same JWTs the
//! WebSocket accepts (interactive logins and API-key tokens from identity's
//! `POST /v1/api-keys/token`). Every command goes through the hub functions
//! the WebSocket handlers use ([`Hub::authorize_trade`], [`Hub::place_order`],
//! [`Hub::cancel_order`], ...), so validation, the account claim, the
//! read-only scope and the per-account order rate limit are identical; this
//! module only translates JSON. Errors are `{"error": {"code", "message"}}`
//! with a matching HTTP status. Prices and quantities are decimal strings
//! (numbers are accepted on input); quantities are base units, as on the
//! WebSocket (1 lot = the symbol's `contract_size`).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use client_proto::{Decimal, ErrorCode, OrderUpdate, TimeInForce, Timeframe};
use core_engine::api::{DealQuery, OrderKind, OrderModify, Protection};
use domain::{Fixed, Side};
use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

use crate::auth::Claims;
use crate::hub::{CmdError, Hub, NewOrder};

/// Largest accepted request body.
const MAX_BODY: usize = 64 * 1024;
/// Deal history page size: default and upper bound.
const DEALS_DEFAULT: usize = 100;
const DEALS_MAX: usize = 1_000;
/// Candles per request: default and upper bound (as on the WebSocket).
const CANDLES_DEFAULT: usize = 500;
const CANDLES_MAX: usize = 5_000;
/// Longest client order id (`[A-Za-z0-9._:-]`, URL safe).
const CLIENT_ID_MAX: usize = 64;

/// `/api/v1/*` routes (state: the shared [`Hub`]).
pub fn router() -> Router<Arc<Hub>> {
    let v1 = Router::new()
        .route("/accounts", get(accounts))
        .route("/accounts/{id}", get(account))
        .route("/positions", get(positions))
        .route("/positions/{id}", patch(modify_position))
        .route("/positions/{id}/close", post(close_position))
        .route("/orders", get(orders).post(place_order))
        .route("/orders/{id}", patch(modify_order).delete(cancel_order))
        .route("/deals", get(deals))
        .route("/symbols", get(symbols))
        .route("/quotes/{symbol}", get(quote))
        .route("/candles/{symbol}", get(candles))
        .fallback(|| async {
            ApiError::new(StatusCode::NOT_FOUND, "not_found", "no such endpoint")
        })
        .method_not_allowed_fallback(|| async {
            ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "method not allowed on this endpoint",
            )
        })
        .layer(DefaultBodyLimit::max(MAX_BODY));
    Router::new().nest("/api/v1", v1)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// JSON error response: `{"error": {"code": "...", "message": "..."}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    fn unauthenticated(message: &str) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthenticated", message)
    }
}

/// HTTP status and error code string of a protocol [`ErrorCode`].
pub fn status_of(code: ErrorCode) -> (StatusCode, &'static str) {
    match code {
        ErrorCode::Unspecified | ErrorCode::BadRequest | ErrorCode::UnsupportedVersion => {
            (StatusCode::BAD_REQUEST, "bad_request")
        }
        ErrorCode::Unauthenticated => (StatusCode::UNAUTHORIZED, "unauthenticated"),
        ErrorCode::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
        ErrorCode::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        ErrorCode::UnknownSymbol => (StatusCode::BAD_REQUEST, "unknown_symbol"),
        ErrorCode::UnknownOrder => (StatusCode::NOT_FOUND, "unknown_order"),
        ErrorCode::InsufficientMargin => (StatusCode::UNPROCESSABLE_ENTITY, "insufficient_margin"),
        ErrorCode::OrderRejected => (StatusCode::UNPROCESSABLE_ENTITY, "order_rejected"),
        ErrorCode::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        ErrorCode::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

impl From<CmdError> for ApiError {
    fn from(CmdError(code, message): CmdError) -> Self {
        let (status, code) = status_of(code);
        ApiError {
            status,
            code,
            message,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({"error": {"code": self.code, "message": self.message}});
        let mut r = (self.status, Json(body)).into_response();
        let h = r.headers_mut();
        if self.status == StatusCode::UNAUTHORIZED {
            h.insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        if self.status == StatusCode::TOO_MANY_REQUESTS {
            h.insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        }
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        r
    }
}

type ApiResult<T = Response> = Result<T, ApiError>;

fn ok(status: StatusCode, body: Value) -> ApiResult {
    let mut r = (status, Json(body)).into_response();
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(r)
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

/// Verified bearer token. Also applies the per-key REST request rate limit.
pub struct Authed(pub Claims);

/// The token of an `Authorization: Bearer <token>` header (scheme case-insensitive).
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.trim().split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

impl FromRequestParts<Arc<Hub>> for Authed {
    type Rejection = ApiError;

    async fn from_request_parts(p: &mut Parts, hub: &Arc<Hub>) -> Result<Self, ApiError> {
        let Some(token) = bearer(&p.headers) else {
            hub.metrics.auth_failures.inc();
            return Err(ApiError::unauthenticated("bearer token required"));
        };
        let claims = hub.auth.verify(token).map_err(|e| {
            hub.metrics.auth_failures.inc();
            tracing::debug!(error = %e, "REST auth rejected");
            ApiError::unauthenticated("invalid token")
        })?;
        if !hub.allow_rest(&claims) {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "request rate limit exceeded",
            ));
        }
        hub.metrics.rest_requests.inc();
        Ok(Authed(claims))
    }
}

/// The account a request targets: the given one, else the token's only
/// account. Authorization is checked by the caller ([`read_account`] or
/// [`Hub::authorize_trade`]).
fn target_account(claims: &Claims, given: Option<&str>) -> ApiResult<String> {
    match given.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => Ok(a.to_string()),
        None => match claims.accounts.as_slice() {
            [only] => Ok(only.clone()),
            [] => Err(ApiError::forbidden("the token holds no trading account")),
            _ => Err(ApiError::bad_request(
                "account required: the token holds several accounts",
            )),
        },
    }
}

/// [`target_account`] for reads: the token must hold the account.
fn read_account(claims: &Claims, given: Option<&str>) -> ApiResult<String> {
    let a = target_account(claims, given)?;
    if !claims.may_access(&a) {
        return Err(ApiError::forbidden("account not authorized"));
    }
    Ok(a)
}

fn need_core(hub: &Hub) -> ApiResult<()> {
    match hub.core() {
        Some(_) => Ok(()),
        None => Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "trading unavailable",
        )),
    }
}

// ---------------------------------------------------------------------------
// Input helpers
// ---------------------------------------------------------------------------

type Params = Result<Query<HashMap<String, String>>, QueryRejection>;

fn params(q: Params) -> ApiResult<HashMap<String, String>> {
    q.map(|Query(m)| m)
        .map_err(|e| ApiError::bad_request(e.body_text()))
}

fn param<'a>(p: &'a HashMap<String, String>, k: &str) -> Option<&'a str> {
    p.get(k)
        .map(String::as_str)
        .filter(|v| !v.trim().is_empty())
}

/// JSON body; an empty body reads as `{}` (all fields optional endpoints).
fn body<T: DeserializeOwned>(bytes: &[u8]) -> ApiResult<T> {
    let bytes: &[u8] = if bytes.iter().all(u8::is_ascii_whitespace) {
        b"{}"
    } else {
        bytes
    };
    serde_json::from_slice(bytes).map_err(|e| ApiError::bad_request(format!("invalid body: {e}")))
}

/// Decimal from a JSON string (`"1.0855"`) or number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Num(pub Fixed);

impl<'de> Deserialize<'de> for Num {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = match Value::deserialize(d)? {
            Value::String(s) => s,
            Value::Number(n) => n.to_string(),
            _ => return Err(D::Error::custom("expected a decimal string or number")),
        };
        s.trim()
            .parse()
            .map(Num)
            .map_err(|e| D::Error::custom(format!("invalid decimal {s:?}: {e}")))
    }
}

/// Point in time from unix milliseconds (number or digit string) or RFC 3339
/// (`2026-10-01T12:00:00Z`, `2026-10-01T15:00:00+03:00`, `2026-10-01`); ns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Time(pub u64);

impl<'de> Deserialize<'de> for Time {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = match Value::deserialize(d)? {
            Value::String(s) => s,
            Value::Number(n) => n.to_string(),
            _ => return Err(D::Error::custom("expected unix milliseconds or RFC 3339")),
        };
        parse_time(&s)
            .map(Time)
            .ok_or_else(|| D::Error::custom(format!("invalid time {s:?}")))
    }
}

/// PATCH field: absent (keep), `null` (remove) or a value (set).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Field<T> {
    #[default]
    Absent,
    Null,
    Value(T),
}

impl<T> Field<T> {
    fn is_absent(&self) -> bool {
        matches!(self, Field::Absent)
    }
}

impl Field<Num> {
    /// The new value given the current one.
    fn apply(self, current: Option<Fixed>) -> Option<Fixed> {
        match self {
            Field::Absent => current,
            Field::Null => None,
            Field::Value(v) => Some(v.0),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Field<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match Option::<T>::deserialize(d)? {
            Some(v) => Field::Value(v),
            None => Field::Null,
        })
    }
}

fn digits(s: &str) -> Option<i64> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Unix milliseconds or RFC 3339 -> ns since the epoch (`None` = invalid or
/// before 1970).
pub fn parse_time(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(ms) = digits(s) {
        return u64::try_from(ms).ok()?.checked_mul(1_000_000);
    }
    let date = s.get(..10)?;
    let b = date.as_bytes();
    if b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, mo, d) = (
        digits(&date[..4])?,
        digits(&date[5..7])?,
        digits(&date[8..])?,
    );
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&mo) || d < 1 || d > month_days[(mo - 1) as usize] {
        return None;
    }
    let mut secs = days_from_civil(y, mo, d) * 86_400;
    let mut nanos = 0i64;
    let rest = &s[10..];
    if !rest.is_empty() {
        let rest = rest.strip_prefix(['T', 't', ' '])?;
        let hms = rest.get(..8)?;
        let hb = hms.as_bytes();
        if hb[2] != b':' || hb[5] != b':' {
            return None;
        }
        let (h, mi, se) = (digits(&hms[..2])?, digits(&hms[3..5])?, digits(&hms[6..])?);
        if h > 23 || mi > 59 || se > 60 {
            return None;
        }
        secs += h * 3_600 + mi * 60 + se;
        let mut zone = &rest[8..];
        if let Some(f) = zone.strip_prefix('.') {
            let n = f.bytes().take_while(u8::is_ascii_digit).count();
            if n == 0 || n > 9 {
                return None;
            }
            nanos = digits(&f[..n])? * 10i64.pow(9 - n as u32);
            zone = &f[n..];
        }
        match zone {
            "Z" | "z" => {}
            z if z.len() == 6 && (z.starts_with('+') || z.starts_with('-')) => {
                if z.as_bytes()[3] != b':' {
                    return None;
                }
                let (oh, om) = (digits(&z[1..3])?, digits(&z[4..])?);
                if oh > 23 || om > 59 {
                    return None;
                }
                let off = oh * 3_600 + om * 60;
                secs -= if z.starts_with('+') { off } else { -off };
            }
            _ => return None,
        }
    }
    let ns = secs.checked_mul(1_000_000_000)?.checked_add(nanos)?;
    u64::try_from(ns).ok()
}

static CLIENT_SEQ: AtomicU64 = AtomicU64::new(0);

/// Client order id from the body, else the `Idempotency-Key` header, else a
/// generated one (`api-<ns>-<seq>`: no idempotency across retries).
fn client_order_id(given: Option<String>, headers: &HeaderMap) -> ApiResult<String> {
    let header = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    match given.or(header).map(|s| s.trim().to_string()) {
        Some(id) => {
            let ok = !id.is_empty()
                && id.len() <= CLIENT_ID_MAX
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'));
            if ok {
                Ok(id)
            } else {
                Err(ApiError::bad_request(format!(
                    "client_order_id: 1-{CLIENT_ID_MAX} characters of A-Z a-z 0-9 . _ : -"
                )))
            }
        }
        None => Ok(format!(
            "api-{}-{}",
            domain::now_ns(),
            CLIENT_SEQ.fetch_add(1, Ordering::Relaxed)
        )),
    }
}

fn side(s: &str) -> ApiResult<Side> {
    match s.trim().to_ascii_lowercase().as_str() {
        "buy" => Ok(Side::Buy),
        "sell" => Ok(Side::Sell),
        _ => Err(ApiError::bad_request("side must be buy or sell")),
    }
}

fn order_kind(s: Option<&str>) -> ApiResult<OrderKind> {
    match s.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        None | Some("market") => Ok(OrderKind::Market),
        Some("limit") => Ok(OrderKind::Limit),
        Some("stop") => Ok(OrderKind::Stop),
        Some("stop_limit") => Ok(OrderKind::StopLimit),
        _ => Err(ApiError::bad_request(
            "type must be market, limit, stop or stop_limit",
        )),
    }
}

fn tif(s: Option<&str>) -> ApiResult<Option<TimeInForce>> {
    Ok(match s.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        None => None,
        Some("gtc") => Some(TimeInForce::Gtc),
        Some("gtd") => Some(TimeInForce::Gtd),
        Some("ioc") => Some(TimeInForce::Ioc),
        Some("fok") => Some(TimeInForce::Fok),
        Some("day") => Some(TimeInForce::Day),
        _ => {
            return Err(ApiError::bad_request(
                "tif must be gtc, gtd, ioc, fok or day",
            ))
        }
    })
}

// ---------------------------------------------------------------------------
// Output shapes
// ---------------------------------------------------------------------------

fn dec(d: Option<Decimal>) -> Option<String> {
    d.and_then(Decimal::to_fixed).map(|f| f.to_string())
}

fn opt_id(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

fn side_name(v: i32) -> &'static str {
    match client_proto::Side::try_from(v) {
        Ok(client_proto::Side::Buy) => "buy",
        Ok(client_proto::Side::Sell) => "sell",
        _ => "unspecified",
    }
}

fn order_type_name(v: i32) -> &'static str {
    use client_proto::OrderType as T;
    match T::try_from(v) {
        Ok(T::Market) => "market",
        Ok(T::Limit) => "limit",
        Ok(T::Stop) => "stop",
        Ok(T::StopLimit) => "stop_limit",
        _ => "unspecified",
    }
}

fn status_name(v: i32) -> &'static str {
    use client_proto::OrderStatus as S;
    match S::try_from(v) {
        Ok(S::PendingNew) => "pending_new",
        Ok(S::New) => "new",
        Ok(S::PartiallyFilled) => "partially_filled",
        Ok(S::Filled) => "filled",
        Ok(S::Canceled) => "canceled",
        Ok(S::Replaced) => "replaced",
        Ok(S::Rejected) => "rejected",
        Ok(S::Expired) => "expired",
        _ => "unspecified",
    }
}

fn margin_mode_name(v: i32) -> &'static str {
    match client_proto::MarginMode::try_from(v) {
        Ok(client_proto::MarginMode::Netting) => "netting",
        Ok(client_proto::MarginMode::Hedging) => "hedging",
        _ => "unspecified",
    }
}

#[derive(Debug, Serialize)]
pub struct ApiAccount {
    pub id: String,
    pub currency: String,
    pub group: String,
    pub margin_mode: &'static str,
    pub leverage: u32,
    pub balance: Option<String>,
    pub equity: Option<String>,
    pub margin: Option<String>,
    pub free_margin: Option<String>,
    /// Equity / margin in percent; `null` without margin in use.
    pub margin_level: Option<String>,
    pub open_positions: usize,
}

impl ApiAccount {
    fn new(info: &client_proto::AccountInfo, s: &client_proto::AccountSnapshot) -> Self {
        ApiAccount {
            id: s.account_id.clone(),
            currency: s.currency.clone(),
            group: info.group.clone(),
            margin_mode: margin_mode_name(s.margin_mode),
            leverage: s.leverage,
            balance: dec(s.balance),
            equity: dec(s.equity),
            margin: dec(s.margin_used),
            free_margin: dec(s.free_margin),
            margin_level: dec(s.margin_level),
            open_positions: s.positions.len(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ApiPosition {
    pub id: String,
    pub account: String,
    pub symbol: String,
    pub side: &'static str,
    /// Absolute size in base units.
    pub qty: Option<String>,
    /// Signed size (positive = long).
    pub net_qty: Option<String>,
    pub avg_price: Option<String>,
    pub unrealized_pnl: Option<String>,
    pub sl: Option<String>,
    pub tp: Option<String>,
    pub trailing_distance: Option<String>,
    pub open_time_ns: u64,
    /// The TP / SL rests at the liquidity provider as a real order.
    pub lp_tp: bool,
    pub lp_sl: bool,
}

impl ApiPosition {
    fn new(account: &str, p: &client_proto::Position) -> Self {
        ApiPosition {
            id: p.position_id.clone(),
            account: account.to_string(),
            symbol: p.symbol.clone(),
            side: side_name(p.side),
            qty: dec(p.qty),
            net_qty: dec(p.net_qty),
            avg_price: dec(p.avg_price),
            unrealized_pnl: dec(p.unrealized_pnl),
            sl: dec(p.sl),
            tp: dec(p.tp),
            trailing_distance: dec(p.trailing_distance),
            open_time_ns: p.open_time_ns,
            lp_tp: p.lp_tp,
            lp_sl: p.lp_sl,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ApiOrder {
    pub id: String,
    pub client_order_id: String,
    pub account: String,
    pub symbol: String,
    pub side: &'static str,
    #[serde(rename = "type")]
    pub order_type: &'static str,
    pub status: &'static str,
    pub qty: Option<String>,
    pub filled_qty: Option<String>,
    pub leaves_qty: Option<String>,
    pub avg_price: Option<String>,
    pub limit_price: Option<String>,
    pub stop_price: Option<String>,
    pub sl: Option<String>,
    pub tp: Option<String>,
    pub trailing_distance: Option<String>,
    pub oco_group: Option<u64>,
    pub expire_at_ns: Option<u64>,
    /// Position opened or increased by the order.
    pub position_id: Option<String>,
    /// Position the order closes.
    pub close_position_id: Option<String>,
    pub stop_triggered: bool,
    /// Pending entry resting at the liquidity provider.
    pub lp_resting: bool,
    /// Reject / cancel reason.
    pub text: Option<String>,
    pub created_ns: u64,
    pub updated_ns: u64,
}

impl ApiOrder {
    pub fn new(o: &OrderUpdate) -> Self {
        ApiOrder {
            id: o.order_id.clone(),
            client_order_id: o.client_request_id.clone(),
            account: o.account_id.clone(),
            symbol: o.symbol.clone(),
            side: side_name(o.side),
            order_type: order_type_name(o.order_type),
            status: status_name(o.status),
            qty: dec(o.qty),
            filled_qty: dec(o.filled_qty),
            leaves_qty: dec(o.leaves_qty),
            avg_price: dec(o.avg_price),
            limit_price: dec(o.limit_price),
            stop_price: dec(o.stop_price),
            sl: dec(o.sl),
            tp: dec(o.tp),
            trailing_distance: dec(o.trailing_distance),
            oco_group: (o.oco_group != 0).then_some(o.oco_group),
            expire_at_ns: (o.expire_at_ns != 0).then_some(o.expire_at_ns),
            position_id: opt_id(&o.position_id),
            close_position_id: opt_id(&o.close_position_id),
            stop_triggered: o.stop_triggered,
            lp_resting: o.lp_resting,
            text: opt_id(&o.text),
            created_ns: o.created_ns,
            updated_ns: o.ts_ns,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ApiDeal {
    pub id: String,
    pub account: String,
    pub order_id: String,
    pub client_order_id: String,
    pub position_id: String,
    pub symbol: String,
    pub side: &'static str,
    /// `in` (opens / increases) or `out` (closes / reduces).
    pub entry: &'static str,
    pub qty: Option<String>,
    pub price: Option<String>,
    pub realized_pnl: Option<String>,
    pub commission: Option<String>,
    /// `client`, `stop_loss`, `take_profit` or `stop_out`.
    pub reason: &'static str,
    pub ts_ns: u64,
}

impl ApiDeal {
    fn new(account: &str, d: &client_proto::Deal) -> Self {
        use client_proto::{DealEntry as E, DealReason as R};
        ApiDeal {
            id: d.deal_id.clone(),
            account: account.to_string(),
            order_id: d.order_id.clone(),
            client_order_id: d.client_request_id.clone(),
            position_id: d.position_id.clone(),
            symbol: d.symbol.clone(),
            side: side_name(d.side),
            entry: match E::try_from(d.entry) {
                Ok(E::In) => "in",
                Ok(E::Out) => "out",
                _ => "unspecified",
            },
            qty: dec(d.qty),
            price: dec(d.price),
            realized_pnl: dec(d.realized_pnl),
            commission: dec(d.commission),
            reason: match R::try_from(d.reason) {
                Ok(R::Client) => "client",
                Ok(R::StopLoss) => "stop_loss",
                Ok(R::TakeProfit) => "take_profit",
                Ok(R::StopOut) => "stop_out",
                _ => "unspecified",
            },
            ts_ns: d.ts_ns,
        }
    }
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

async fn accounts(State(hub): State<Arc<Hub>>, Authed(c): Authed) -> ApiResult {
    need_core(&hub)?;
    let mut out = Vec::new();
    for a in &c.accounts {
        if let Some((info, snap)) = hub.account_state(a).await {
            out.push(ApiAccount::new(&info, &snap));
        }
    }
    ok(StatusCode::OK, json!({ "accounts": out }))
}

async fn account(
    State(hub): State<Arc<Hub>>,
    Authed(c): Authed,
    Path(id): Path<String>,
) -> ApiResult {
    let id = read_account(&c, Some(&id))?;
    need_core(&hub)?;
    let (info, snap) = hub.account_state(&id).await.ok_or_else(|| {
        ApiError::new(StatusCode::NOT_FOUND, "unknown_account", "unknown account")
    })?;
    ok(
        StatusCode::OK,
        json!({ "account": ApiAccount::new(&info, &snap) }),
    )
}

async fn positions(State(hub): State<Arc<Hub>>, Authed(c): Authed, q: Params) -> ApiResult {
    let p = params(q)?;
    let account = read_account(&c, param(&p, "account"))?;
    need_core(&hub)?;
    let snap = hub.account_snapshot(&account).await.ok_or_else(|| {
        ApiError::new(StatusCode::NOT_FOUND, "unknown_account", "unknown account")
    })?;
    let out: Vec<_> = snap
        .positions
        .iter()
        .map(|p| ApiPosition::new(&account, p))
        .collect();
    ok(
        StatusCode::OK,
        json!({ "account": account, "positions": out }),
    )
}

fn flag(p: &HashMap<String, String>, k: &str) -> bool {
    param(p, k).is_some_and(|v| matches!(v.trim(), "1" | "true" | "yes"))
}

async fn orders(State(hub): State<Arc<Hub>>, Authed(c): Authed, q: Params) -> ApiResult {
    let p = params(q)?;
    let account = read_account(&c, param(&p, "account"))?;
    let list = if flag(&p, "history") {
        hub.order_history(&account).await?
    } else {
        hub.orders(&account).await?
    };
    let out: Vec<_> = list.iter().map(ApiOrder::new).collect();
    ok(StatusCode::OK, json!({ "account": account, "orders": out }))
}

fn time_param(p: &HashMap<String, String>, k: &str) -> ApiResult<u64> {
    match param(p, k) {
        None => Ok(0),
        Some(v) => parse_time(v).ok_or_else(|| {
            ApiError::bad_request(format!("{k}: unix milliseconds or RFC 3339 expected"))
        }),
    }
}

fn limit_param(p: &HashMap<String, String>, default: usize, max: usize) -> ApiResult<usize> {
    match param(p, "limit") {
        None => Ok(default),
        Some(v) => match v.trim().parse::<usize>() {
            Ok(n) if n > 0 => Ok(n.min(max)),
            _ => Err(ApiError::bad_request("limit must be a positive integer")),
        },
    }
}

async fn deals(State(hub): State<Arc<Hub>>, Authed(c): Authed, q: Params) -> ApiResult {
    let p = params(q)?;
    let account = read_account(&c, param(&p, "account"))?;
    let after = match param(&p, "cursor") {
        None => 0,
        Some(v) => v
            .trim()
            .parse::<u64>()
            .map_err(|_| ApiError::bad_request("bad cursor"))?,
    };
    let query = DealQuery {
        from_ns: time_param(&p, "from")?,
        to_ns: time_param(&p, "to")?,
        after,
        limit: limit_param(&p, DEALS_DEFAULT, DEALS_MAX)?,
    };
    let page = hub.deals(&account, query).await?;
    let out: Vec<_> = page
        .deals
        .iter()
        .map(|d| ApiDeal::new(&account, &crate::hub::deal(d)))
        .collect();
    ok(
        StatusCode::OK,
        json!({
            "account": account,
            "deals": out,
            "next_cursor": page.next.map(|n| n.to_string()),
        }),
    )
}

async fn symbols(State(hub): State<Arc<Hub>>, Authed(_): Authed) -> ApiResult {
    let out: Vec<_> = hub
        .instruments()
        .into_iter()
        .map(|i| {
            json!({
                "symbol": i.symbol,
                "base": i.base,
                "quote": i.quote,
                "digits": i.digits,
                "tick_size": dec(i.tick_size),
                "qty_step": dec(i.qty_step),
                "contract_size": dec(i.contract_size),
            })
        })
        .collect();
    ok(StatusCode::OK, json!({ "symbols": out }))
}

fn unknown_symbol(symbol: &str) -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "unknown_symbol",
        format!("unknown symbol {symbol}"),
    )
}

async fn quote(
    State(hub): State<Arc<Hub>>,
    Authed(c): Authed,
    Path(symbol): Path<String>,
    q: Params,
) -> ApiResult {
    let p = params(q)?;
    if !hub.is_known_symbol(&symbol) {
        return Err(unknown_symbol(&symbol));
    }
    // Quotes are marked up per group: the price of the given account's group,
    // else of the token's accounts (as a WebSocket session sees them).
    let accounts = match param(&p, "account") {
        Some(a) => vec![read_account(&c, Some(a))?],
        None => c.accounts.clone(),
    };
    let groups: HashSet<String> = accounts
        .iter()
        .filter_map(|a| hub.account_group(a))
        .collect();
    // accounts in several groups see different mark-ups: the caller must say
    // which account the price is for, or get a price it cannot trade on
    if groups.len() > 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "account_required",
            "the token holds accounts in several groups: pass ?account=",
        ));
    }
    let q = hub
        .last_quotes(std::slice::from_ref(&symbol), &groups)
        .pop()
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "no_quote",
                format!("no quote for {symbol} yet"),
            )
        })?;
    ok(
        StatusCode::OK,
        json!({
            "symbol": &*q.symbol,
            "bid": q.bid.map(|f| f.to_string()),
            "ask": q.ask.map(|f| f.to_string()),
            "ts_ns": q.ts_ns,
        }),
    )
}

async fn candles(
    State(hub): State<Arc<Hub>>,
    Authed(_): Authed,
    Path(symbol): Path<String>,
    q: Params,
) -> ApiResult {
    let p = params(q)?;
    if !hub.is_known_symbol(&symbol) {
        return Err(unknown_symbol(&symbol));
    }
    let tf = param(&p, "timeframe")
        .and_then(|t| Timeframe::from_str_name(&t.trim().to_ascii_uppercase()))
        .filter(|t| *t != Timeframe::Unspecified)
        .ok_or_else(|| ApiError::bad_request("timeframe must be M1, M5, M15, M30, H1, H4 or D1"))?;
    let bars = hub.candles(
        &symbol,
        tf,
        time_param(&p, "from")?,
        time_param(&p, "to")?,
        limit_param(&p, CANDLES_DEFAULT, CANDLES_MAX)?,
    );
    let out: Vec<_> = bars
        .iter()
        .map(|b| {
            json!({
                "open_time_ns": b.open_time_ns,
                "open": dec(b.open),
                "high": dec(b.high),
                "low": dec(b.low),
                "close": dec(b.close),
                "ticks": b.ticks,
            })
        })
        .collect();
    ok(
        StatusCode::OK,
        json!({ "symbol": symbol, "timeframe": tf.as_str_name(), "candles": out }),
    )
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceBody {
    #[serde(default)]
    account: Option<String>,
    symbol: String,
    side: String,
    #[serde(rename = "type", default)]
    order_type: Option<String>,
    qty: Num,
    #[serde(default)]
    limit_price: Option<Num>,
    #[serde(default)]
    stop_price: Option<Num>,
    #[serde(default)]
    sl: Option<Num>,
    #[serde(default)]
    tp: Option<Num>,
    #[serde(default)]
    trailing_distance: Option<Num>,
    #[serde(default)]
    tif: Option<String>,
    #[serde(default)]
    expire_at: Option<Time>,
    #[serde(default)]
    oco_group: Option<u64>,
    #[serde(default)]
    max_deviation_points: Option<u32>,
    #[serde(default)]
    client_order_id: Option<String>,
}

/// A command whose client order id the engine already holds: answer from the
/// stored order when it is the same request (`same`), else 409.
async fn replay(
    hub: &Hub,
    account: &str,
    clid: &str,
    same: impl Fn(&OrderUpdate) -> bool,
) -> ApiResult {
    let found = hub
        .find_order(account, |o| o.client_request_id == clid)
        .await?;
    match found {
        Some(o) if same(&o) => ok(
            StatusCode::OK,
            json!({
                "order_id": o.order_id,
                "client_order_id": clid,
                "account": account,
                "replayed": true,
                "order": ApiOrder::new(&o),
            }),
        ),
        Some(_) => Err(ApiError::new(
            StatusCode::CONFLICT,
            "client_order_id_conflict",
            "client_order_id already used for a different order",
        )),
        None => Err(ApiError::new(
            StatusCode::CONFLICT,
            "duplicate_client_order_id",
            "client_order_id already used",
        )),
    }
}

async fn place_order(
    State(hub): State<Arc<Hub>>,
    Authed(c): Authed,
    headers: HeaderMap,
    bytes: Bytes,
) -> ApiResult {
    // Malformed requests are refused before they spend the order budget; the
    // order itself is validated by the hub, as for the WebSocket.
    let b: PlaceBody = body(&bytes)?;
    let account = target_account(&c, b.account.as_deref())?;
    let clid = client_order_id(b.client_order_id, &headers)?;
    let side = side(&b.side)?;
    let ord_type = order_kind(b.order_type.as_deref())?;
    let tif = tif(b.tif.as_deref())?;
    hub.authorize_trade(&c, &account)?;
    let order = NewOrder {
        request_id: clid.clone(),
        account_id: account.clone(),
        symbol: b.symbol.trim().to_string(),
        side,
        ord_type,
        qty: b.qty.0,
        limit_price: b.limit_price.map(|n| n.0),
        tif,
        stop_price: b.stop_price.map(|n| n.0),
        sl: b.sl.map(|n| n.0),
        tp: b.tp.map(|n| n.0),
        trailing_distance: b.trailing_distance.map(|n| n.0),
        oco_group: b.oco_group.filter(|g| *g != 0),
        expire_at_ns: b.expire_at.map(|t| t.0),
        max_deviation_points: b.max_deviation_points.filter(|d| *d != 0),
    };
    let symbol = order.symbol.clone();
    let (qty, limit, stop, kind) = (
        order.qty,
        order.limit_price,
        order.stop_price,
        order.ord_type,
    );
    match hub.place_order(order).await {
        Ok(order_id) => ok(
            StatusCode::CREATED,
            json!({
                "order_id": order_id.to_string(),
                "client_order_id": clid,
                "account": account,
                "replayed": false,
            }),
        ),
        Err(e) if e.is_duplicate_order() => {
            let side = client_proto::Side::from_domain(side) as i32;
            // a replay only when it is the SAME order; a reused id with other
            // terms is a conflict, not a silent success
            let kind = crate::hub::order_type(kind) as i32;
            replay(&hub, &account, &clid, |o| {
                o.symbol == symbol
                    && o.side == side
                    && o.close_position_id.is_empty()
                    && o.order_type == kind
                    && o.qty.and_then(Decimal::to_fixed) == Some(qty)
                    && o.limit_price.and_then(Decimal::to_fixed) == limit
                    && o.stop_price.and_then(Decimal::to_fixed) == stop
            })
            .await
        }
        Err(e) => Err(e.into()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountBody {
    #[serde(default)]
    account: Option<String>,
}

/// The account of a command addressed by path id: `?account=` or a JSON
/// `account` field, else the token's only account.
fn command_account(
    c: &Claims,
    p: &HashMap<String, String>,
    b: Option<String>,
) -> ApiResult<String> {
    target_account(c, param(p, "account").map(str::to_string).or(b).as_deref())
}

async fn cancel_order(
    State(hub): State<Arc<Hub>>,
    Authed(c): Authed,
    Path(id): Path<String>,
    q: Params,
    bytes: Bytes,
) -> ApiResult {
    let p = params(q)?;
    let b: AccountBody = body(&bytes)?;
    let account = command_account(&c, &p, b.account)?;
    hub.authorize_trade(&c, &account)?;
    // `{id}` is the client order id or the engine order id of a working order.
    let working = hub.orders(&account).await?;
    let clid = working
        .iter()
        .find(|o| o.client_request_id == id)
        .or_else(|| working.iter().find(|o| o.order_id == id))
        .map_or(id, |o| o.client_request_id.clone());
    hub.cancel_order(&account, &clid).await?;
    ok(
        StatusCode::OK,
        json!({ "cancelled": true, "client_order_id": clid, "account": account }),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModifyOrderBody {
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    qty: Option<Num>,
    #[serde(default)]
    limit_price: Option<Num>,
    #[serde(default)]
    stop_price: Option<Num>,
    #[serde(default)]
    sl: Field<Num>,
    #[serde(default)]
    tp: Field<Num>,
    #[serde(default)]
    trailing_distance: Field<Num>,
    #[serde(default)]
    expire_at: Field<Time>,
}

async fn modify_order(
    State(hub): State<Arc<Hub>>,
    Authed(c): Authed,
    Path(id): Path<String>,
    q: Params,
    bytes: Bytes,
) -> ApiResult {
    let p = params(q)?;
    let b: ModifyOrderBody = body(&bytes)?;
    let account = command_account(&c, &p, b.account.clone())?;
    hub.authorize_trade(&c, &account)?;
    let working = hub.orders(&account).await?;
    let cur = working
        .iter()
        .find(|o| o.client_request_id == id)
        .or_else(|| working.iter().find(|o| o.order_id == id))
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "unknown_order",
                "no pending order to modify",
            )
        })?;
    let mut change = OrderModify {
        qty: b.qty.map(|n| n.0),
        limit_price: b.limit_price.map(|n| n.0),
        stop_price: b.stop_price.map(|n| n.0),
        ..Default::default()
    };
    // `null` removes a level: the engine then takes the full set, the
    // omitted ones at their current value.
    let replace = [b.sl, b.tp, b.trailing_distance].contains(&Field::Null);
    let base = |d: Option<Decimal>| {
        if replace {
            d.and_then(Decimal::to_fixed)
        } else {
            None
        }
    };
    change.replace_protection = replace;
    change.sl = b.sl.apply(base(cur.sl));
    change.tp = b.tp.apply(base(cur.tp));
    change.trailing_distance = b.trailing_distance.apply(base(cur.trailing_distance));
    match b.expire_at {
        Field::Absent => {}
        Field::Null => change.clear_expiry = true,
        Field::Value(t) => change.expire_at_ns = Some(t.0),
    }
    if change == OrderModify::default() {
        return Err(ApiError::bad_request("nothing to change"));
    }
    let clid = cur.client_request_id.clone();
    hub.modify_order(&account, &clid, change).await?;
    ok(
        StatusCode::OK,
        json!({ "modified": true, "client_order_id": clid, "account": account }),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModifyPositionBody {
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    sl: Field<Num>,
    #[serde(default)]
    tp: Field<Num>,
    #[serde(default)]
    trailing_distance: Field<Num>,
}

async fn modify_position(
    State(hub): State<Arc<Hub>>,
    Authed(c): Authed,
    Path(id): Path<String>,
    q: Params,
    bytes: Bytes,
) -> ApiResult {
    let p = params(q)?;
    let b: ModifyPositionBody = body(&bytes)?;
    let account = command_account(&c, &p, b.account.clone())?;
    hub.authorize_trade(&c, &account)?;
    if [b.sl, b.tp, b.trailing_distance]
        .iter()
        .all(Field::is_absent)
    {
        return Err(ApiError::bad_request(
            "nothing to change: give sl, tp and/or trailing_distance",
        ));
    }
    need_core(&hub)?;
    // The engine replaces all three levels: omitted fields keep their value.
    let unknown = || {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown_position",
            "unknown position",
        )
    };
    let snap = hub.account_snapshot(&account).await.ok_or_else(unknown)?;
    let cur = snap
        .positions
        .iter()
        .find(|p| p.position_id == id)
        .ok_or_else(unknown)?;
    let fx = |d: Option<Decimal>| d.and_then(Decimal::to_fixed);
    let prot = Protection {
        sl: b.sl.apply(fx(cur.sl)),
        tp: b.tp.apply(fx(cur.tp)),
        trailing_distance: b.trailing_distance.apply(fx(cur.trailing_distance)),
    };
    hub.modify_position(&account, &id, prot).await?;
    let s = |f: Option<Fixed>| f.map(|f| f.to_string());
    ok(
        StatusCode::OK,
        json!({
            "position_id": id,
            "account": account,
            "sl": s(prot.sl),
            "tp": s(prot.tp),
            "trailing_distance": s(prot.trailing_distance),
        }),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseBody {
    #[serde(default)]
    account: Option<String>,
    /// Base units to close; all when omitted.
    #[serde(default)]
    qty: Option<Num>,
    #[serde(default)]
    client_order_id: Option<String>,
}

async fn close_position(
    State(hub): State<Arc<Hub>>,
    Authed(c): Authed,
    Path(id): Path<String>,
    q: Params,
    headers: HeaderMap,
    bytes: Bytes,
) -> ApiResult {
    let p = params(q)?;
    let b: CloseBody = body(&bytes)?;
    let account = command_account(&c, &p, b.account)?;
    let clid = client_order_id(b.client_order_id, &headers)?;
    hub.authorize_trade(&c, &account)?;
    match hub
        .close_position(&account, &id, b.qty.map(|n| n.0), &clid)
        .await
    {
        Ok(order_id) => ok(
            StatusCode::CREATED,
            json!({
                "order_id": order_id.to_string(),
                "client_order_id": clid,
                "account": account,
                "position_id": id,
                "replayed": false,
            }),
        ),
        Err(e) if e.is_duplicate_order() => {
            replay(&hub, &account, &clid, |o| o.close_position_id == id).await
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_parse_as_unix_ms_or_rfc3339() {
        assert_eq!(parse_time("0"), Some(0));
        assert_eq!(parse_time("1700000000123"), Some(1_700_000_000_123_000_000));
        assert_eq!(parse_time("1970-01-01"), Some(0));
        assert_eq!(
            parse_time("2000-03-01T00:00:00Z"),
            Some(951_868_800_000_000_000)
        );
        assert_eq!(
            parse_time("2026-10-08T12:30:15.5Z"),
            Some(1_791_462_615_500_000_000)
        );
        // offsets move the instant back to UTC
        assert_eq!(
            parse_time("2026-10-08T15:30:15+03:00"),
            parse_time("2026-10-08T12:30:15Z")
        );
        assert_eq!(
            parse_time("2026-10-08T10:30:15-02:00"),
            parse_time("2026-10-08T12:30:15Z")
        );
        assert_eq!(parse_time("2024-02-29"), parse_time("2024-02-29T00:00:00z"));
        for bad in [
            "",
            "-5",
            "2023-02-29",
            "2026-13-01",
            "2026-10-08T25:00:00Z",
            "2026-10-08T12:00:00",
            "2026-10-08T12:00:00+0300",
            "1969-12-31",
            "yesterday",
            "2026-1-8",
        ] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn decimals_and_patch_fields() {
        #[derive(Deserialize)]
        struct B {
            #[serde(default)]
            a: Field<Num>,
            #[serde(default)]
            b: Field<Num>,
            #[serde(default)]
            c: Field<Num>,
            n: Num,
        }
        let b: B = serde_json::from_str(r#"{"a": null, "b": "1.08505", "n": 100000}"#).unwrap();
        assert_eq!(b.a, Field::Null);
        assert_eq!(b.b, Field::Value(Num("1.08505".parse().unwrap())));
        assert_eq!(b.c, Field::Absent);
        assert_eq!(b.n.0, Fixed::from_int(100_000));
        let cur = Some(Fixed::from_int(1));
        assert_eq!(b.a.apply(cur), None);
        assert_eq!(b.c.apply(cur), cur);
        assert_eq!(b.b.apply(cur), Some("1.08505".parse().unwrap()));
        // floats are read through their decimal text; junk is an error
        let f: Num = serde_json::from_str("1.5").unwrap();
        assert_eq!(f.0, "1.5".parse().unwrap());
        assert!(serde_json::from_str::<Num>(r#""1.2.3""#).is_err());
        assert!(serde_json::from_str::<Num>("true").is_err());
    }

    #[test]
    fn client_order_ids() {
        let mut h = HeaderMap::new();
        assert_eq!(client_order_id(Some("my-1".into()), &h).unwrap(), "my-1");
        h.insert("idempotency-key", HeaderValue::from_static("k:2"));
        assert_eq!(client_order_id(None, &h).unwrap(), "k:2");
        // the body wins over the header
        assert_eq!(client_order_id(Some("b".into()), &h).unwrap(), "b");
        let long = "x".repeat(65);
        for bad in ["", "a b", "x~y", "ü", long.as_str()] {
            let e = client_order_id(Some(bad.to_string()), &HeaderMap::new()).unwrap_err();
            assert_eq!(e.status, StatusCode::BAD_REQUEST, "{bad}");
        }
        let a = client_order_id(None, &HeaderMap::new()).unwrap();
        let b = client_order_id(None, &HeaderMap::new()).unwrap();
        assert!(a.starts_with("api-") && a != b);
    }

    #[test]
    fn bearer_header() {
        let mut h = HeaderMap::new();
        assert_eq!(bearer(&h), None);
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc"),
        );
        assert_eq!(bearer(&h), Some("abc"));
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer  xyz "),
        );
        assert_eq!(bearer(&h), Some("xyz"));
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert_eq!(bearer(&h), None);
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer "));
        assert_eq!(bearer(&h), None);
    }

    #[test]
    fn account_resolution() {
        let claims = |accounts: &[&str]| Claims {
            sub: "u".into(),
            exp: 0,
            accounts: accounts.iter().map(|s| s.to_string()).collect(),
            roles: vec![],
            amr: vec![],
            scope: None,
            sid: None,
        };
        let one = claims(&["A1"]);
        assert_eq!(read_account(&one, None).unwrap(), "A1");
        assert_eq!(read_account(&one, Some("A1")).unwrap(), "A1");
        assert_eq!(
            read_account(&one, Some("B9")).unwrap_err().status,
            StatusCode::FORBIDDEN
        );
        let two = claims(&["A1", "A2"]);
        assert_eq!(
            read_account(&two, None).unwrap_err().status,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(read_account(&two, Some(" A2 ")).unwrap(), "A2");
        assert_eq!(
            read_account(&claims(&[]), None).unwrap_err().status,
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn error_codes_map_to_http_status() {
        assert_eq!(
            status_of(ErrorCode::Forbidden),
            (StatusCode::FORBIDDEN, "forbidden")
        );
        assert_eq!(
            status_of(ErrorCode::RateLimited).0,
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            status_of(ErrorCode::InsufficientMargin).0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(status_of(ErrorCode::UnknownOrder).0, StatusCode::NOT_FOUND);
        assert_eq!(
            status_of(ErrorCode::Unavailable).0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        let e: ApiError = CmdError(ErrorCode::OrderRejected, "SL above bid".into()).into();
        assert_eq!(
            (e.status, e.code, e.message.as_str()),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "order_rejected",
                "SL above bid"
            )
        );
    }
}
