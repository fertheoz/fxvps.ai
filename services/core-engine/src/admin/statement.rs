//! Client account statements: the data of `GET /v1/client/statement`, its
//! plain HTML rendering and the monthly statement e-mail.
//!
//! Monthly e-mail (plan item 8), OFF unless `CORE_STATEMENT_EMAIL=1`: on the
//! 1st to 3rd day of a month (UTC) the engine renders last month's statement
//! for every client account whose profile has a deliverable e-mail address
//! and sends it over the platform's SMTP channel (`CORE_SMTP_URL` /
//! `CORE_MAIL_FROM`, falling back to identity's `IDENTITY_SMTP_URL` /
//! `IDENTITY_MAIL_FROM`; Resend's SMTP relay works as is). Every mailed
//! account is journaled in the admin store (`StatementMailed`), so a restart
//! never mails the same month twice; a pass that left failures is retried at
//! most [`MAX_PASSES`] times. Mail bodies, addresses and SMTP credentials are
//! never logged.

use super::auth::Actor;
use super::store::{AdminCmd, AdminState, AdminUserRec, StatementMonth};
use super::{need, AdminCtx, ApiError, ApiResult};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use lettre::message::{Mailbox, MultiPart};
use lettre::transport::stub::AsyncStubTransport;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use oms::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

const DAY_NS: u64 = 86_400_000_000_000;
/// Statements go out on days 1..=RUN_DAYS of the month (catch-up after downtime).
pub const RUN_DAYS: u32 = 3;
/// A month with failed sends is retried at most this many passes.
pub const MAX_PASSES: u32 = 3;
const TICK: Duration = Duration::from_secs(30 * 60);

/// `CORE_STATEMENT_EMAIL=1` (or `true`) turns the monthly e-mail on.
pub fn enabled() -> bool {
    std::env::var("CORE_STATEMENT_EMAIL").is_ok_and(|v| v == "1" || v == "true")
}

// ---------------------------------------------------------------- data

/// Statement bodies of `logins` for [from, to): [`super::routes::statement_body`]
/// with each account's cash rows (the same body the terminal shows).
pub(super) fn engine_bodies(
    e: &Engine,
    logins: &[(u64, Vec<(u64, Value)>)],
    from: u64,
    to: u64,
) -> Vec<(u64, Option<Value>)> {
    logins
        .iter()
        .map(|(l, cash)| {
            (
                *l,
                super::routes::statement_body(e, *l, from, to, cash.clone()),
            )
        })
        .collect()
}

/// Adds the client name and the broker name to a statement body.
pub(super) fn complete(mut body: Value, st: &AdminState, login: u64, fallback_name: &str) -> Value {
    body["name"] = json!(st
        .profiles
        .get(&login)
        .map_or(fallback_name, |p| p.name.as_str()));
    body["broker"] = json!(st.settings.broker_name);
    body
}

// ---------------------------------------------------------------- schedule

/// Statement period `YYYY-MM` and its [start, end) for a run at `now`: the
/// previous calendar month (UTC).
pub fn last_month(now: u64) -> (String, u64, u64) {
    let (this, last) = super::routes::month_start(now);
    let (y, m, _) = super::routes::civil_from_days((last / DAY_NS) as i64);
    (format!("{y:04}-{m:02}"), last, this)
}

/// [`last_month`] when a run is due at `now` (days 1..=[`RUN_DAYS`]).
pub fn due_month(now: u64) -> Option<(String, u64, u64)> {
    let (_, _, d) = super::routes::civil_from_days((now / DAY_NS) as i64);
    (d <= RUN_DAYS).then(|| last_month(now))
}

/// A real mailbox: one `@`, a dotted domain, not a reserved test domain.
pub fn deliverable(email: &str) -> bool {
    let e = email.trim().to_ascii_lowercase();
    let Some((local, domain)) = e.split_once('@') else {
        return false;
    };
    if local.is_empty() || domain.contains('@') || !domain.contains('.') {
        return false;
    }
    let reserved = ["example.com", "example.org", "example.net"];
    let reserved_tld = [".test", ".invalid", ".example", ".localhost", ".local"];
    !reserved
        .iter()
        .any(|r| domain == *r || domain.ends_with(&format!(".{r}")))
        && !reserved_tld.iter().any(|t| domain.ends_with(t))
}

/// Client accounts with a deliverable profile address.
pub fn recipients(st: &AdminState) -> Vec<(u64, String)> {
    st.profiles
        .iter()
        .filter(|(_, p)| deliverable(&p.email))
        .map(|(a, p)| (*a, p.email.trim().to_string()))
        .collect()
}

/// Accounts still to be mailed for `month`; `None` when the month is done.
/// Idempotent across restarts: it only reads the journaled admin state.
pub fn pending(st: &AdminState, month: &str) -> Option<Vec<(u64, String)>> {
    let run = st.statement_mail.get(month);
    if run.is_some_and(|r| r.done) {
        return None;
    }
    Some(
        recipients(st)
            .into_iter()
            .filter(|(a, _)| !run.is_some_and(|r| r.sent.contains(a)))
            .collect(),
    )
}

/// Starts the monthly run (no-op unless [`enabled`] and SMTP is configured).
pub fn spawn(ctx: AdminCtx) {
    if !enabled() {
        return;
    }
    let Some(mailer) = ctx.mailer.clone() else {
        tracing::warn!("CORE_STATEMENT_EMAIL=1 but no SMTP is configured: statements are not sent");
        return;
    };
    tracing::info!("monthly statement e-mail on");
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(TICK);
        loop {
            iv.tick().await;
            match run_once(&ctx, &mailer, super::routes::now_ns()).await {
                Ok(Some((month, sent, failed))) => {
                    tracing::info!(%month, sent, failed, "monthly statements pass")
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e.message, "monthly statements pass failed"),
            }
        }
    });
}

/// One pass: mails every pending account of the due month, journals each
/// success and the pass. `None` when nothing was due.
async fn run_once(
    ctx: &AdminCtx,
    mailer: &Mailer,
    now: u64,
) -> Result<Option<(String, u32, u32)>, ApiError> {
    let Some((month, from, to)) = due_month(now) else {
        return Ok(None);
    };
    let Some(todo) = pending(&ctx.store.lock().await.state, &month) else {
        return Ok(None);
    };
    let st = ctx.view_state().await;
    let logins: Vec<(u64, Vec<(u64, Value)>)> = todo
        .iter()
        .map(|(a, _)| (*a, super::routes::ops_cash(&st, *a, from, to)))
        .collect();
    let bodies = ctx.qr(move |e| engine_bodies(e, &logins, from, to)).await?;
    let (mut sent, mut failed) = (0u32, 0u32);
    for ((account, to_addr), (_, body)) in todo.iter().zip(bodies) {
        // accounts gone from the engine are skipped, not retried
        let Some(body) = body else { continue };
        let s = complete(body, &st, *account, "");
        let subj = subject(&s, &month, false);
        match mailer
            .send(
                to_addr,
                &subj,
                render_text(&s, &month),
                render_html(&s, &month),
            )
            .await
        {
            Ok(()) => {
                sent += 1;
                ctx.store.lock().await.append(
                    &Actor::system(),
                    AdminCmd::StatementMailed {
                        month: month.clone(),
                        account: *account,
                    },
                )?;
            }
            Err(e) => {
                failed += 1;
                tracing::warn!(account, %month, error = %e, "statement e-mail failed");
            }
        }
    }
    ctx.store.lock().await.append(
        &Actor::system(),
        AdminCmd::StatementPass {
            month: month.clone(),
            sent,
            failed,
        },
    )?;
    ctx.notify(&["listAudit", "getStatementMail"]);
    Ok(Some((month, sent, failed)))
}

// ---------------------------------------------------------------- rendering

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Minor units -> `-1,234.56 USD` (grouped, the currency's own decimals).
pub fn fmt_money(minor: i64, ccy: &str) -> String {
    let code = <[u8; 3]>::try_from(ccy.as_bytes()).unwrap_or(*b"USD");
    let digits = money::Currency::new(code).minor_exponent();
    let s = money::format_scaled(i128::from(minor), digits);
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", s.as_str()),
    };
    let (int, frac) = match rest.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rest, None),
    };
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{grouped}.{f} {ccy}"),
        None => format!("{sign}{grouped} {ccy}"),
    }
}

/// `2026-09-30T21:04:05.000Z` -> `2026-09-30 21:04`.
fn when(v: &Value) -> String {
    let s = v.as_str().unwrap_or("");
    s.get(..16).unwrap_or(s).replace('T', " ")
}

fn num(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

fn lots(v: &Value) -> String {
    format!("{:.2}", v.as_f64().unwrap_or(0.0))
}

fn subject(s: &Value, month: &str, test: bool) -> String {
    format!(
        "{}{} account statement {month} - #{}",
        if test { "[TEST] " } else { "" },
        s["broker"].as_str().unwrap_or("fxvps.ai"),
        s["account"]
    )
}

fn cash_totals(s: &Value) -> (i64, i64) {
    let rows = s["cash"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let sum = |k: &str| {
        rows.iter()
            .filter(|r| r["kind"] == k)
            .map(|r| num(&r["amount"]))
            .sum::<i64>()
    };
    (sum("deposit"), sum("withdraw"))
}

/// Plain-text alternative: the summary only.
pub fn render_text(s: &Value, month: &str) -> String {
    let ccy = s["currency"].as_str().unwrap_or("USD");
    let m = |v: &Value| fmt_money(num(v), ccy);
    let t = &s["totals"];
    let (dep, wd) = cash_totals(s);
    format!(
        "{broker} - account statement {month}\nAccount #{acc} - {name}\nPeriod: {from} - {to} (UTC)\n\n\
         Balance: {bal}\nEquity: {eq}\nMargin: {mar}\n\n\
         Closed trades: {n} ({lots} lots)\nP&L: {pnl}\nCommission: {com}\nSwap: {swap}\n\
         Deposits: {dep}\nWithdrawals: {wd}\nOpen positions: {open}\n\n\
         The full statement is in the HTML part of this message.\n",
        broker = s["broker"].as_str().unwrap_or(""),
        acc = s["account"],
        name = s["name"].as_str().unwrap_or(""),
        from = when(&s["from"]),
        to = when(&s["to"]),
        bal = m(&s["balance"]),
        eq = m(&s["equity"]),
        mar = m(&s["margin"]),
        n = num(&t["trades"]),
        lots = lots(&t["lots"]),
        pnl = m(&t["pnl"]),
        com = m(&t["commission"]),
        swap = m(&t["swap"]),
        dep = fmt_money(dep, ccy),
        wd = fmt_money(wd, ccy),
        open = s["positions"].as_array().map_or(0, Vec::len),
    )
}

fn table(head: &[&str], rows: Vec<Vec<String>>, empty: &str) -> String {
    let th: String = head
        .iter()
        .map(|h| format!("<th style=\"text-align:left;padding:4px 8px;border-bottom:1px solid #ccc\">{}</th>", esc(h)))
        .collect();
    let body: String = if rows.is_empty() {
        format!(
            "<tr><td colspan=\"{}\" style=\"padding:4px 8px;color:#777\">{}</td></tr>",
            head.len(),
            esc(empty)
        )
    } else {
        rows.iter()
            .map(|r| {
                let tds: String = r
                    .iter()
                    .map(|c| {
                        format!(
                            "<td style=\"padding:4px 8px;border-bottom:1px solid #eee\">{}</td>",
                            esc(c)
                        )
                    })
                    .collect();
                format!("<tr>{tds}</tr>")
            })
            .collect()
    };
    format!("<table style=\"border-collapse:collapse;width:100%;font-size:13px\"><thead><tr>{th}</tr></thead><tbody>{body}</tbody></table>")
}

/// The statement as a self-contained HTML page (inline styles only, every
/// value escaped). Same data as `GET /v1/client/statement`.
pub fn render_html(s: &Value, month: &str) -> String {
    let ccy = s["currency"].as_str().unwrap_or("USD");
    let m = |v: &Value| fmt_money(num(v), ccy);
    let t = &s["totals"];
    let (dep, wd) = cash_totals(s);
    let rows = |k: &str| s[k].as_array().cloned().unwrap_or_default();
    let summary = table(
        &["", ""],
        vec![
            vec!["Balance / Bakiye".into(), m(&s["balance"])],
            vec!["Equity / Varlık".into(), m(&s["equity"])],
            vec!["Margin / Teminat".into(), m(&s["margin"])],
            vec![
                "Closed trades / Kapanan işlemler".into(),
                format!("{} ({} lots)", num(&t["trades"]), lots(&t["lots"])),
            ],
            vec!["P&L / Kâr-zarar".into(), m(&t["pnl"])],
            vec!["Commission / Komisyon".into(), m(&t["commission"])],
            vec!["Swap".into(), m(&t["swap"])],
            vec!["Deposits / Yatırma".into(), fmt_money(dep, ccy)],
            vec!["Withdrawals / Çekme".into(), fmt_money(wd, ccy)],
        ],
        "",
    );
    let trades = table(
        &[
            "Time (UTC)",
            "Symbol",
            "Side",
            "Lots",
            "Price",
            "P&L",
            "Commission",
            "Swap",
        ],
        rows("trades")
            .iter()
            .map(|r| {
                vec![
                    when(&r["at"]),
                    r["symbol"].as_str().unwrap_or("").into(),
                    r["side"].as_str().unwrap_or("").into(),
                    lots(&r["lots"]),
                    r["price"].to_string(),
                    m(&r["pnl"]),
                    m(&r["commission"]),
                    m(&r["swap"]),
                ]
            })
            .collect(),
        "No closed trades in this period.",
    );
    let cash = table(
        &["Time (UTC)", "Type", "Amount", "Reason"],
        rows("cash")
            .iter()
            .map(|r| {
                vec![
                    when(&r["at"]),
                    r["kind"].as_str().unwrap_or("").into(),
                    m(&r["amount"]),
                    r["reason"].as_str().unwrap_or("").into(),
                ]
            })
            .collect(),
        "No deposits or withdrawals in this period.",
    );
    let positions = table(
        &[
            "#",
            "Symbol",
            "Side",
            "Lots",
            "Open price",
            "Opened (UTC)",
            "Swap",
        ],
        rows("positions")
            .iter()
            .map(|r| {
                vec![
                    r["id"].to_string(),
                    r["symbol"].as_str().unwrap_or("").into(),
                    r["side"].as_str().unwrap_or("").into(),
                    lots(&r["lots"]),
                    r["openPrice"].to_string(),
                    when(&r["openedAt"]),
                    m(&r["swap"]),
                ]
            })
            .collect(),
        "No open positions.",
    );
    let h2 = |x: &str| {
        format!(
            "<h2 style=\"font-size:15px;margin:20px 0 6px\">{}</h2>",
            esc(x)
        )
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head>\
         <body style=\"font-family:Arial,Helvetica,sans-serif;color:#111;max-width:760px;margin:0 auto;padding:16px\">\
         <h1 style=\"font-size:18px;margin:0 0 4px\">{broker}</h1>\
         <div style=\"font-size:14px\">Account statement / Hesap ekstresi <b>{month}</b></div>\
         <div style=\"font-size:13px;color:#444;margin-top:6px\">#{acc} &middot; {name} &middot; {group} &middot; {ccy}<br>\
         {from} &ndash; {to} (UTC) &middot; generated {gen}</div>\
         {h_sum}{summary}{h_tr}{trades}{h_cash}{cash}{h_pos}{positions}\
         <p style=\"font-size:11px;color:#777;margin-top:20px\">Amounts in {ccy}. Balance, equity and open positions are \
         as of the generation time. This statement was generated automatically; reply to your broker's support for questions.</p>\
         </body></html>",
        title = esc(&format!("Statement {month} #{}", s["account"])),
        broker = esc(s["broker"].as_str().unwrap_or("")),
        month = esc(month),
        acc = s["account"],
        name = esc(s["name"].as_str().unwrap_or("")),
        group = esc(s["group"].as_str().unwrap_or("")),
        ccy = esc(ccy),
        from = esc(&when(&s["from"])),
        to = esc(&when(&s["to"])),
        gen = esc(&when(&s["generatedAt"])),
        h_sum = h2("Summary / Özet"),
        h_tr = h2("Closed trades / Kapanan işlemler"),
        h_cash = h2("Deposits and withdrawals / Yatırma ve çekme"),
        h_pos = h2("Open positions / Açık pozisyonlar"),
    )
}

// ---------------------------------------------------------------- mail

enum Transport {
    Smtp(AsyncSmtpTransport<Tokio1Executor>),
    Stub(AsyncStubTransport),
}

/// Statement mailer over the platform's SMTP relay (or a stub in tests).
pub struct Mailer {
    transport: Transport,
    from: Mailbox,
}

impl std::fmt::Debug for Mailer {
    // never the transport: its URL may carry credentials
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mailer")
            .field("from", &self.from.to_string())
            .finish_non_exhaustive()
    }
}

fn env_any(keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
}

impl Mailer {
    /// From `CORE_SMTP_URL` + `CORE_MAIL_FROM`, else identity's
    /// `IDENTITY_SMTP_URL` + `IDENTITY_MAIL_FROM`; `None` when unset or invalid
    /// (the error is logged without the URL).
    pub fn from_env() -> Option<Mailer> {
        let url = env_any(&["CORE_SMTP_URL", "IDENTITY_SMTP_URL"])?;
        let from = env_any(&["CORE_MAIL_FROM", "IDENTITY_MAIL_FROM"])?;
        let t = match AsyncSmtpTransport::<Tokio1Executor>::from_url(&url) {
            Ok(b) => b.timeout(Some(Duration::from_secs(20))).build(),
            Err(e) => {
                tracing::warn!(error = %e, "statement mail: invalid SMTP URL");
                return None;
            }
        };
        match from.parse() {
            Ok(from) => Some(Mailer {
                transport: Transport::Smtp(t),
                from,
            }),
            Err(e) => {
                tracing::warn!(error = %e, "statement mail: invalid sender address");
                None
            }
        }
    }

    /// Test mailer: messages land in the returned stub.
    pub fn stub(from: &str) -> Result<(Mailer, AsyncStubTransport), String> {
        let stub = AsyncStubTransport::new_ok();
        let m = Mailer {
            transport: Transport::Stub(stub.clone()),
            from: from.parse().map_err(|e| format!("sender: {e}"))?,
        };
        Ok((m, stub))
    }

    pub fn from_address(&self) -> String {
        self.from.email.to_string()
    }

    /// Sends a multipart (plain text + HTML) message.
    pub async fn send(
        &self,
        to: &str,
        subject: &str,
        text: String,
        html: String,
    ) -> Result<(), String> {
        let msg = Message::builder()
            .from(self.from.clone())
            .to(to.parse().map_err(|e| format!("recipient: {e}"))?)
            .subject(subject)
            .multipart(MultiPart::alternative_plain_html(text, html))
            .map_err(|e| e.to_string())?;
        match &self.transport {
            Transport::Smtp(t) => t.send(msg).await.map(|_| ()).map_err(|e| e.to_string()),
            Transport::Stub(t) => t.send(msg).await.map(|_| ()).map_err(|e| e.to_string()),
        }
    }
}

// ---------------------------------------------------------------- console

/// The signed-in staff member's own address: the token's e-mail (identity
/// tokens carry no display name) or the admin user record.
fn actor_email(actor: &Actor, users: &BTreeMap<String, AdminUserRec>) -> Option<String> {
    let address = |s: &str| {
        let s = s.trim();
        (s.contains('@') && !s.contains(char::is_whitespace)).then(|| s.to_lowercase())
    };
    address(&actor.name)
        .or_else(|| {
            users
                .values()
                .find(|u| u.id == actor.sub)
                .and_then(|u| address(&u.email))
        })
        .or_else(|| address(&actor.sub))
}

/// `GET /v1/settings/statement-email`: flag, channel and last month's run.
pub async fn mail_status(State(ctx): State<AdminCtx>, actor: Actor) -> ApiResult {
    need(&actor, "settings.view")?;
    let (month, _, _) = last_month(super::routes::now_ns());
    let (run, recipients) = {
        let st = &ctx.store.lock().await.state;
        (st.statement_mail.get(&month).cloned(), recipients(st).len())
    };
    Ok(Json(json!({
        "enabled": enabled(),
        "configured": ctx.mailer.is_some(),
        "from": ctx.mailer.as_ref().map(|m| m.from_address()),
        "runDays": RUN_DAYS,
        "month": month,
        "recipients": recipients,
        "run": run.map(|r: StatementMonth| json!({
            "sent": r.sent.len(), "passes": r.passes, "failed": r.failed, "done": r.done,
        })),
    })))
}

#[derive(Deserialize, Default)]
pub struct TestReq {
    #[serde(default)]
    account: Option<u64>,
}

/// `POST /v1/settings/statement-email/test`: last month's statement of one
/// client account (default: the lowest with a profile) to the caller's own
/// address. Works with the monthly flag off, so the channel can be checked first.
pub async fn mail_test(
    State(ctx): State<AdminCtx>,
    actor: Actor,
    body: Option<Json<TestReq>>,
) -> ApiResult {
    need(&actor, "settings.edit")?;
    need(&actor, "clients.view")?;
    let req = body.map(|Json(r)| r).unwrap_or_default();
    let mailer = ctx.mailer.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::CONFLICT,
            "mail_not_configured",
            "no SMTP channel: set CORE_SMTP_URL / CORE_MAIL_FROM (or IDENTITY_SMTP_URL / IDENTITY_MAIL_FROM)",
        )
    })?;
    let st = ctx.view_state().await;
    let users = ctx.store.lock().await.state.users.clone();
    let to_addr = actor_email(&actor, &users)
        .ok_or_else(|| ApiError::bad("no e-mail address is known for your user"))?;
    let account = req
        .account
        .or_else(|| st.profiles.keys().next().copied())
        .ok_or_else(|| ApiError::bad("no client account to render; pass {\"account\": <login>}"))?;
    let (month, from, to) = last_month(super::routes::now_ns());
    let cash = super::routes::ops_cash(&st, account, from, to);
    let body = ctx
        .qr(move |e| super::routes::statement_body(e, account, from, to, cash))
        .await?
        .ok_or_else(|| ApiError::not_found("unknown account"))?;
    let s = complete(body, &st, account, "");
    mailer
        .send(
            &to_addr,
            &subject(&s, &month, true),
            render_text(&s, &month),
            render_html(&s, &month),
        )
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "mail_failed", e))?;
    ctx.store.lock().await.append(
        &actor,
        AdminCmd::StatementTestSent {
            month: month.clone(),
            account,
        },
    )?;
    ctx.notify(&["listAudit"]);
    Ok(Json(
        json!({ "ok": true, "to": to_addr, "account": account, "month": month }),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::store::{AdminRecord, ClientProfile};
    use super::super::views;
    use super::*;

    fn ns(y: i64, m: u32, d: u32, h: u64) -> u64 {
        super::super::routes::days_from_civil(y, m, d) as u64 * DAY_NS + h * 3_600_000_000_000
    }

    #[test]
    fn month_key_and_window() {
        let (k, from, to) = due_month(ns(2026, 10, 1, 0)).unwrap();
        assert_eq!(k, "2026-09");
        assert_eq!(views::iso(from), "2026-09-01T00:00:00.000Z");
        assert_eq!(views::iso(to), "2026-10-01T00:00:00.000Z");
        // catch-up through the 3rd, nothing after
        assert_eq!(due_month(ns(2026, 10, 3, 23)).unwrap().0, "2026-09");
        assert!(due_month(ns(2026, 10, 4, 0)).is_none());
        assert!(due_month(ns(2026, 9, 30, 23)).is_none());
        // January statement of December
        assert_eq!(due_month(ns(2027, 1, 2, 5)).unwrap().0, "2026-12");
        assert_eq!(last_month(ns(2026, 3, 15, 0)).0, "2026-02");
    }

    fn rec(seq: u64, cmd: AdminCmd) -> AdminRecord {
        AdminRecord {
            seq,
            ts: seq,
            actor: Actor::system(),
            cmd,
        }
    }

    #[test]
    fn month_run_is_idempotent_across_replay() {
        let mut st = AdminState::default();
        for (a, email) in [
            (1u64, "ali@mail.com"),
            (2, "veli@mail.com"),
            (3, "account3@example.com"),
            (4, "nobody"),
        ] {
            st.profiles.insert(
                a,
                ClientProfile {
                    name: format!("c{a}"),
                    email: email.into(),
                    lei: None,
                },
            );
        }
        let all = pending(&st, "2026-09").unwrap();
        assert_eq!(all.iter().map(|r| r.0).collect::<Vec<_>>(), vec![1, 2]);
        // account 1 mailed, then a pass with one failure: only 2 is left
        let mut records = vec![
            rec(
                1,
                AdminCmd::StatementMailed {
                    month: "2026-09".into(),
                    account: 1,
                },
            ),
            rec(
                2,
                AdminCmd::StatementPass {
                    month: "2026-09".into(),
                    sent: 1,
                    failed: 1,
                },
            ),
        ];
        for r in &records {
            st.apply(r);
        }
        assert_eq!(
            pending(&st, "2026-09").unwrap(),
            vec![(2, "veli@mail.com".to_string())]
        );
        // another month is independent
        assert_eq!(pending(&st, "2026-10").unwrap().len(), 2);
        // the retry mails 2 and closes the month
        records.push(rec(
            3,
            AdminCmd::StatementMailed {
                month: "2026-09".into(),
                account: 2,
            },
        ));
        records.push(rec(
            4,
            AdminCmd::StatementPass {
                month: "2026-09".into(),
                sent: 1,
                failed: 0,
            },
        ));
        for r in &records[2..] {
            st.apply(r);
        }
        assert!(pending(&st, "2026-09").is_none());
        // a restart replays the journal into the same answer
        let mut replayed = AdminState {
            profiles: st.profiles.clone(),
            ..Default::default()
        };
        for r in &records {
            replayed.apply(r);
        }
        assert!(pending(&replayed, "2026-09").is_none());
        assert_eq!(replayed.statement_mail["2026-09"].sent.len(), 2);
    }

    #[test]
    fn failing_month_gives_up_after_max_passes() {
        let mut st = AdminState::default();
        for seq in 1..=u64::from(MAX_PASSES) {
            assert!(pending(&st, "2026-09").is_some());
            st.apply(&rec(
                seq,
                AdminCmd::StatementPass {
                    month: "2026-09".into(),
                    sent: 0,
                    failed: 2,
                },
            ));
        }
        assert!(pending(&st, "2026-09").is_none());
        assert_eq!(st.statement_mail["2026-09"].passes, MAX_PASSES);
    }

    #[test]
    fn deliverable_addresses() {
        assert!(deliverable("Saygin@Mail.com"));
        assert!(!deliverable("x@example.com"));
        assert!(!deliverable("x@sub.example.org"));
        assert!(!deliverable("x@host.test"));
        assert!(!deliverable("x@localhost"));
        assert!(!deliverable("@mail.com"));
        assert!(!deliverable("a@b@mail.com"));
    }

    #[test]
    fn staff_address_for_the_test_mail() {
        let mut a = Actor::system();
        a.name = "Ops@Fxvps.ai".into();
        assert_eq!(
            actor_email(&a, &BTreeMap::new()).as_deref(),
            Some("ops@fxvps.ai")
        );
        // a display name is not an address: the user record is
        a.name = "ops@x display".into();
        a.sub = "u1".into();
        let users = BTreeMap::from([(
            "u1".to_string(),
            AdminUserRec {
                id: "u1".into(),
                name: "Ops".into(),
                email: "ops@fxvps.ai".into(),
                role: super::super::auth::Role::Admin,
                mfa: true,
                active: true,
                last_login: None,
                tenant: None,
            },
        )]);
        assert_eq!(actor_email(&a, &users).as_deref(), Some("ops@fxvps.ai"));
        a.sub = "u2".into();
        assert_eq!(actor_email(&a, &users), None);
    }

    #[test]
    fn money_formatting() {
        assert_eq!(fmt_money(123_456_789, "USD"), "1,234,567.89 USD");
        assert_eq!(fmt_money(-50, "EUR"), "-0.50 EUR");
        assert_eq!(fmt_money(1_000, "JPY"), "1,000 JPY");
        assert_eq!(fmt_money(0, "USD"), "0.00 USD");
    }

    fn sample() -> Value {
        json!({
            "account": 100001, "group": "demo-retail", "currency": "USD",
            "from": "2026-09-01T00:00:00.000Z", "to": "2026-09-30T23:59:59.999Z",
            "generatedAt": "2026-10-01T00:30:00.000Z",
            "balance": 123_456, "equity": 130_000, "margin": 1_000,
            "trades": [{"at": "2026-09-10T08:15:00.000Z", "symbol": "EURUSD", "side": "buy", "lots": 1.5,
                        "price": 1.1001, "pnl": 25_000, "commission": -700, "swap": -120, "position": 7}],
            "totals": {"trades": 1, "lots": 1.5, "pnl": 25_000, "commission": -700, "swap": -120},
            "positions": [{"id": 9, "symbol": "XAUUSD", "side": "sell", "lots": 0.1, "openPrice": 2650.5,
                           "openedAt": "2026-09-29T12:00:00.000Z", "swap": -35}],
            "cash": [{"at": "2026-09-02T10:00:00.000Z", "kind": "deposit", "amount": 500_000, "reason": "wire <b>in</b>"},
                     {"at": "2026-09-20T10:00:00.000Z", "kind": "withdraw", "amount": 20_000, "reason": "out"}],
            "name": "Saygın <script>alert(1)</script>",
            "broker": "fxvps & co",
        })
    }

    #[test]
    fn renders_escaped_html_with_statement_data() {
        let h = render_html(&sample(), "2026-09");
        assert!(h.starts_with("<!doctype html>"));
        // every user-controlled value is escaped
        assert!(!h.contains("<script>") && h.contains("&lt;script&gt;"));
        assert!(h.contains("fxvps &amp; co"));
        assert!(h.contains("wire &lt;b&gt;in&lt;/b&gt;"));
        assert!(h.contains("Saygın"));
        // amounts in the account currency, grouped
        for want in [
            "1,234.56 USD",
            "250.00 USD",
            "-7.00 USD",
            "5,000.00 USD",
            "200.00 USD",
            "EURUSD",
            "1.50",
            "2026-09-10 08:15",
            "XAUUSD",
            "#100001",
            "2026-09",
        ] {
            assert!(h.contains(want), "missing {want}");
        }
        let t = render_text(&sample(), "2026-09");
        assert!(t.contains("Balance: 1,234.56 USD"));
        assert!(t.contains("Deposits: 5,000.00 USD"));
        assert!(t.contains("Withdrawals: 200.00 USD"));
        assert!(t.contains("Closed trades: 1 (1.50 lots)"));
        assert_eq!(
            subject(&sample(), "2026-09", true),
            "[TEST] fxvps & co account statement 2026-09 - #100001"
        );
        // empty sections say so
        let mut s = sample();
        s["trades"] = json!([]);
        assert!(render_html(&s, "2026-09").contains("No closed trades in this period."));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn monthly_pass_resumes_and_never_resends() {
        use money::{px, Currency, Money};
        use risk::{GroupConfig, Routing, SymbolSpec};
        let dir = tempfile::tempdir().unwrap();
        let (h, join) = crate::spawn(crate::Settings::new(dir.path())).unwrap();
        let mut cmds = vec![
            oms::Command::AddSymbol(SymbolSpec::fx("EURUSD", Currency::EUR, Currency::USD, 5)),
            oms::Command::SetGroup(GroupConfig::retail("b", Currency::USD, Routing::BBook)),
            oms::Command::Quote {
                symbol: "EURUSD".into(),
                bid: px("1.1"),
                ask: px("1.1001"),
            },
        ];
        for a in [7u64, 8] {
            cmds.push(oms::Command::OpenAccount {
                account: a,
                group: "b".into(),
            });
            cmds.push(oms::Command::Deposit {
                account: a,
                amount: Money::parse("1000", Currency::USD).unwrap(),
                key: format!("d{a}"),
            });
        }
        for c in cmds {
            h.command(c).await.unwrap();
        }
        let mut store = super::super::store::AdminStore::open(dir.path()).unwrap();
        // 9 has a profile but no engine account: skipped, not retried
        for (a, email) in [(7u64, "a@mail.com"), (8, "b@mail.com"), (9, "c@mail.com")] {
            store
                .append(
                    &Actor::system(),
                    AdminCmd::AccountOpened {
                        account: a,
                        group: "b".into(),
                        profile: ClientProfile {
                            name: format!("Client {a}"),
                            email: email.into(),
                            lei: None,
                        },
                    },
                )
                .unwrap();
        }
        // the statement period is the current month: run on the 1st of the next one
        let real = super::super::routes::now_ns();
        let (y, m, _) = super::super::routes::civil_from_days((real / DAY_NS) as i64);
        let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
        let at = ns(ny, nm, 1, 2);
        let (month, _, _) = due_month(at).unwrap();
        // a previous process mailed 7 before it stopped
        store
            .append(
                &Actor::system(),
                AdminCmd::StatementMailed {
                    month: month.clone(),
                    account: 7,
                },
            )
            .unwrap();
        let (mailer, stub) = Mailer::stub("statements@fxvps.test").unwrap();
        let ctx = AdminCtx {
            fix_log_dir: std::path::PathBuf::from("/nonexistent"),
            engine: h.clone(),
            store: std::sync::Arc::new(tokio::sync::Mutex::new(store)),
            auth: std::sync::Arc::new(super::super::auth::Authenticator::hs256(b"statement-test")),
            live: tokio::sync::broadcast::channel(16).0,
            tickets: std::sync::Arc::default(),
            lp_status: None,
            lp_admin: None,
            agg: None,
            alerts: std::sync::Arc::default(),
            names: None,
            data_dir: dir.path().to_path_buf(),
            replica: None,
            http: reqwest::Client::new(),
            mailer: None,
            hazine: None,
        };
        let r = run_once(&ctx, &mailer, at).await.unwrap();
        assert_eq!(r, Some((month.clone(), 1, 0)));
        let sent = stub.messages().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0.to()[0].to_string(), "b@mail.com");
        // the month is closed: later ticks and a restart send nothing
        assert_eq!(
            run_once(&ctx, &mailer, at + 3_600_000_000_000)
                .await
                .unwrap(),
            None
        );
        assert_eq!(stub.messages().await.len(), 1);
        let reopened = super::super::store::AdminStore::open(dir.path()).unwrap();
        let run = &reopened.state.statement_mail[&month];
        assert!(run.done);
        assert_eq!(run.sent.iter().copied().collect::<Vec<_>>(), vec![7, 8]);
        // outside the window nothing is due
        assert_eq!(
            run_once(&ctx, &mailer, ns(ny, nm, 4, 0)).await.unwrap(),
            None
        );
        h.shutdown();
        join.join().unwrap();
    }

    #[tokio::test]
    async fn stub_mailer_sends_multipart() {
        let (m, stub) = Mailer::stub("fxvps <statements@fxvps.test>").unwrap();
        assert_eq!(m.from_address(), "statements@fxvps.test");
        m.send("client@mail.com", "S", "plain".into(), "<b>html</b>".into())
            .await
            .unwrap();
        let sent = stub.messages().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0.to()[0].to_string(), "client@mail.com");
        assert!(sent[0].1.contains("multipart/alternative"));
        assert!(m
            .send("bad", "S", String::new(), String::new())
            .await
            .is_err());
        // Debug never shows the transport
        assert!(!format!("{m:?}").contains("Stub"));
    }
}
