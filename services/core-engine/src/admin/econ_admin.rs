//! Economic calendar endpoints. Console: list / create / edit / delete and
//! "import this week" from the free ForexFactory feed (`/v1/econ-calendar*`,
//! `settings.view` / `settings.edit`, journaled as `AdminCmd::EconEvent*`).
//! Terminal: `GET /v1/client/calendar` (through the gateway's `/api/client`).
//!
//! The events live in the admin store, not in the engine, so these reads
//! never touch the writer thread.

use super::auth::Actor;
use super::econ_calendar::{self, EconEvent, Impact};
use super::store::AdminCmd;
use super::{need, views, AdminCtx, ApiError, ApiResult, ClientActor};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

/// This week's calendar (free, public; refreshed by its publisher a few times an hour).
pub const DEFAULT_FEED_URL: &str = "https://nfs.faireconomy.media/ff_calendar_thisweek.json";
const FEED_SOURCE: &str = "forexfactory";
const MAX_FEED_BYTES: usize = 2 * 1024 * 1024;
const DAY_NS: u64 = 86_400_000_000_000;
const MS: u64 = 1_000_000;
/// Longest window one read returns, and the row cap.
const MAX_SPAN_NS: u64 = 400 * DAY_NS;
const MAX_ROWS: usize = 5_000;

/// `CORE_ECON_FEED_URL` overrides the feed (a mirror, or a test server).
fn feed_url() -> String {
    std::env::var("CORE_ECON_FEED_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_FEED_URL.into())
}

pub fn event_json(e: &EconEvent) -> Value {
    json!({
        "id": e.id, "time": e.time_ns / MS, "at": views::iso(e.time_ns),
        "currency": e.currency, "title": e.title, "impact": e.impact,
        "actual": e.actual, "forecast": e.forecast, "previous": e.previous,
    })
}

#[derive(Deserialize)]
pub struct RangeQuery {
    from: Option<String>,
    to: Option<String>,
    /// Comma separated, e.g. `EUR,USD`.
    currency: Option<String>,
}

/// Epoch milliseconds or an ISO-8601 date / date-time -> ns.
fn bound(s: &str) -> Option<u64> {
    let s = s.trim();
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        return s.parse::<u64>().ok()?.checked_mul(MS);
    }
    econ_calendar::parse_rfc3339_ns(s)
}

/// `[from, to)` in ns: default the last 7 days to 14 days ahead, at most 400 days.
fn window(q: &RangeQuery, now: u64) -> Result<(u64, u64), ApiError> {
    let parse = |v: &Option<String>, name: &str| -> Result<Option<u64>, ApiError> {
        match v.as_deref().filter(|s| !s.trim().is_empty()) {
            None => Ok(None),
            Some(s) => bound(s)
                .map(Some)
                .ok_or_else(|| ApiError::bad(format!("invalid {name} {s:?}"))),
        }
    };
    let from = parse(&q.from, "from")?.unwrap_or(now.saturating_sub(7 * DAY_NS));
    let to = parse(&q.to, "to")?.unwrap_or(now.saturating_add(14 * DAY_NS));
    if to <= from {
        return Err(ApiError::bad("to must be after from"));
    }
    Ok((from, to.min(from.saturating_add(MAX_SPAN_NS))))
}

fn currencies(q: &RangeQuery) -> Option<Vec<String>> {
    let list: Vec<String> = q
        .currency
        .as_deref()?
        .split(',')
        .map(|c| c.trim().to_ascii_uppercase())
        .filter(|c| !c.is_empty())
        .collect();
    (!list.is_empty()).then_some(list)
}

async fn read(ctx: &AdminCtx, q: &RangeQuery) -> ApiResult {
    let (from, to) = window(q, super::routes::now_ns())?;
    let ccy = currencies(q);
    let rows = {
        let store = ctx.store.lock().await;
        econ_calendar::in_range(&store.state.econ_events, from, to, ccy.as_deref(), MAX_ROWS)
    };
    Ok(Json(json!({
        "from": from / MS, "to": to / MS,
        "events": rows.iter().map(event_json).collect::<Vec<_>>(),
    })))
}

// ------------------------------------------------------------- console

pub async fn list(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Query(q): Query<RangeQuery>,
) -> ApiResult {
    need(&actor, "settings.view")?;
    read(&ctx, &q).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventReq {
    /// Epoch milliseconds (UTC).
    time: u64,
    currency: String,
    title: String,
    impact: Impact,
    #[serde(default)]
    actual: Option<String>,
    #[serde(default)]
    forecast: Option<String>,
    #[serde(default)]
    previous: Option<String>,
}

fn build(id: String, r: EventReq) -> Result<EconEvent, ApiError> {
    let ev = EconEvent {
        id,
        time_ns: r
            .time
            .checked_mul(MS)
            .ok_or_else(|| ApiError::bad("time out of range"))?,
        currency: r.currency,
        title: r.title,
        impact: r.impact,
        actual: r.actual,
        forecast: r.forecast,
        previous: r.previous,
    }
    .normalized();
    ev.validate().map_err(ApiError::bad)?;
    Ok(ev)
}

fn duplicate(ev: &EconEvent) -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "duplicate",
        format!(
            "{} {} at {} already exists",
            ev.currency,
            ev.title,
            views::iso(ev.time_ns)
        ),
    )
}

pub async fn create(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Json(r): Json<EventReq>,
) -> ApiResult {
    need(&actor, "settings.edit")?;
    let mut store = ctx.store.lock().await;
    let ev = build(format!("ev{}", store.next_seq()), r)?;
    if store
        .state
        .econ_events
        .values()
        .any(|e| e.key() == ev.key())
    {
        return Err(duplicate(&ev));
    }
    store.append(&actor, AdminCmd::EconEventSaved { event: ev.clone() })?;
    drop(store);
    push_news_times(&ctx).await;
    ctx.notify(&["listEconEvents", "listAudit"]);
    Ok(Json(event_json(&ev)))
}

pub async fn update(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
    Json(r): Json<EventReq>,
) -> ApiResult {
    need(&actor, "settings.edit")?;
    let mut store = ctx.store.lock().await;
    if !store.state.econ_events.contains_key(&id) {
        return Err(ApiError::not_found(format!("unknown event {id}")));
    }
    let ev = build(id, r)?;
    if store
        .state
        .econ_events
        .values()
        .any(|e| e.id != ev.id && e.key() == ev.key())
    {
        return Err(duplicate(&ev));
    }
    store.append(&actor, AdminCmd::EconEventSaved { event: ev.clone() })?;
    drop(store);
    push_news_times(&ctx).await;
    ctx.notify(&["listEconEvents", "listAudit"]);
    Ok(Json(event_json(&ev)))
}

pub async fn remove(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    Path(id): Path<String>,
) -> ApiResult {
    need(&actor, "settings.edit")?;
    let mut store = ctx.store.lock().await;
    if !store.state.econ_events.contains_key(&id) {
        return Err(ApiError::not_found(format!("unknown event {id}")));
    }
    store.append(&actor, AdminCmd::EconEventDeleted { id })?;
    drop(store);
    push_news_times(&ctx).await;
    ctx.notify(&["listEconEvents", "listAudit"]);
    Ok(Json(json!({ "ok": true })))
}

fn feed_error(message: String) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "feed_unavailable", message)
}

/// "Import this week": fetches the feed server side and merges it
/// idempotently (same title, currency and time = same event).
pub async fn import(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "settings.edit")?;
    let resp = ctx
        .http
        .get(feed_url())
        .header("user-agent", "fxvps-core-engine (economic calendar)")
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| feed_error(format!("calendar feed: {e}")))?;
    if !resp.status().is_success() {
        return Err(feed_error(format!(
            "calendar feed answered {} (the publisher rate-limits; try again in a few minutes)",
            resp.status()
        )));
    }
    if resp
        .content_length()
        .is_some_and(|n| n > MAX_FEED_BYTES as u64)
    {
        return Err(feed_error("calendar feed is too large".into()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| feed_error(format!("calendar feed: {e}")))?;
    if bytes.len() > MAX_FEED_BYTES {
        return Err(feed_error("calendar feed is too large".into()));
    }
    let parsed =
        econ_calendar::parse_ff_feed(&String::from_utf8_lossy(&bytes)).map_err(feed_error)?;
    let total = parsed.events.len();
    let mut store = ctx.store.lock().await;
    let prefix = format!("ev{}", store.next_seq());
    let m = econ_calendar::merge_import(&store.state.econ_events, parsed.events, &prefix);
    if !m.upserts.is_empty() {
        store.append(
            &actor,
            AdminCmd::EconEventsImported {
                events: m.upserts,
                added: m.added,
                updated: m.updated,
                source: FEED_SOURCE.into(),
            },
        )?;
    }
    drop(store);
    if m.added + m.updated > 0 {
        push_news_times(&ctx).await;
        ctx.notify(&["listEconEvents", "listAudit"]);
    }
    Ok(Json(json!({
        "total": total, "added": m.added, "updated": m.updated,
        "unchanged": m.unchanged, "skipped": parsed.skipped,
    })))
}

// ------------------------------------------------------------- client (terminal)

/// `GET /v1/client/calendar?from&to&currency`: any client token (read-only keys too).
pub async fn client_list(
    State(ctx): State<AdminCtx>,
    _client: ClientActor,
    Query(q): Query<RangeQuery>,
) -> ApiResult {
    read(&ctx, &q).await
}

/// Hands the engine the high-impact events around now (2 h back, 7 days
/// ahead) for the routing rules' news window.
pub async fn push_news_times(ctx: &AdminCtx) {
    let now = domain::now_ns();
    let (from, to) = (
        now.saturating_sub(2 * 3_600_000_000_000),
        now + 7 * 24 * 3_600_000_000_000,
    );
    let times: Vec<u64> = {
        let store = ctx.store.lock().await;
        store
            .state
            .econ_events
            .values()
            .filter(|e| e.impact == Impact::High && (from..=to).contains(&e.time_ns))
            .map(|e| e.time_ns)
            .collect()
    };
    if let Err(e) = ctx.cmd(oms::Command::SetNewsTimes(times)).await {
        tracing::warn!(error = ?e, "news times not handed to the engine");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(from: Option<&str>, to: Option<&str>, currency: Option<&str>) -> RangeQuery {
        RangeQuery {
            from: from.map(Into::into),
            to: to.map(Into::into),
            currency: currency.map(Into::into),
        }
    }

    #[test]
    fn window_bounds() {
        let now = 1_000 * DAY_NS;
        assert_eq!(
            window(&q(None, None, None), now).unwrap(),
            (now - 7 * DAY_NS, now + 14 * DAY_NS)
        );
        // epoch ms and ISO forms
        assert_eq!(
            window(&q(Some("1791504000000"), Some("2026-10-10"), None), now).unwrap(),
            (1_791_504_000_000 * MS, 1_791_590_400_000 * MS)
        );
        // capped span
        let (f, t) = window(&q(Some("0"), Some("9999999999999"), None), now).unwrap();
        assert_eq!((f, t), (0, MAX_SPAN_NS));
        assert!(window(&q(Some("2026-10-10"), Some("2026-10-09"), None), now).is_err());
        assert!(window(&q(Some("yesterday"), None, None), now).is_err());
        assert_eq!(
            currencies(&q(None, None, Some("eur, usd,,"))),
            Some(vec!["EUR".to_string(), "USD".to_string()])
        );
        assert_eq!(currencies(&q(None, None, Some(" , "))), None);
    }
}
