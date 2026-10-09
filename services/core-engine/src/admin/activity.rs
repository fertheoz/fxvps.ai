//! Account behaviour (parça 10a).
//!
//! The client gateway (terminal WebSocket, REST, MT5 bridge) records
//! connection and authentication events here; the admin side scores them
//! together with the engine's orders and deals into per-account flags
//! (scalper, order burst, connect/disconnect churn, brute force, IP hopping)
//! for the console's account-alert box and the `account_abuse` alert.
//!
//! The ring is in memory only (last [`CAPACITY`] events): the flags describe
//! the last `window_h` hours, a restart simply starts a fresh window. Orders
//! and deals come from the journaled engine state and survive restarts.
//!
//! The same handle carries the inbox through which a bridge session hands
//! platform (MT5) user records to the admin store (`PlatformUser`).
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Mutex;

use oms::{DealEntry, Engine, OrderOrigin, OrderStatus};
use serde::{Deserialize, Serialize};

use super::store::{AdminState, BehaviorThresholds, PlatformUser};

pub const CAPACITY: usize = 50_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Connect,
    Disconnect,
    /// Token rejected (terminal / REST).
    AuthFail,
    /// Bridge institution key or IP rejected.
    KeyFail,
    /// Connection limit hit.
    ConnReject,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub ts_ms: u64,
    pub kind: ActivityKind,
    /// Engine account when known (bridge: the institution's account).
    pub account: Option<u64>,
    /// Token subject / external account ids of a terminal session (resolved
    /// to engine accounts by the admin side).
    #[serde(default)]
    pub names: Vec<String>,
    /// `terminal` | `api` | `bridge`
    pub platform: String,
    pub ip: Option<String>,
    pub detail: String,
}

#[derive(Default, Debug)]
pub struct ActivityLog {
    events: Mutex<VecDeque<ActivityEvent>>,
    users_inbox: Mutex<Vec<PlatformUser>>,
}

impl ActivityLog {
    pub fn push(&self, ev: ActivityEvent) {
        let mut q = self.events.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() >= CAPACITY {
            q.pop_front();
        }
        q.push_back(ev);
    }

    /// Records an event now.
    pub fn record(
        &self,
        kind: ActivityKind,
        account: Option<u64>,
        names: Vec<String>,
        platform: &str,
        ip: Option<&str>,
        detail: impl Into<String>,
    ) {
        self.push(ActivityEvent {
            ts_ms: domain::now_ns() / 1_000_000,
            kind,
            account,
            names,
            platform: platform.into(),
            ip: ip.map(str::to_string),
            detail: detail.into(),
        });
    }

    /// Events at or after `ts_ms`, oldest first.
    pub fn since(&self, ts_ms: u64) -> Vec<ActivityEvent> {
        let q = self.events.lock().unwrap_or_else(|e| e.into_inner());
        q.iter().filter(|e| e.ts_ms >= ts_ms).cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Platform user records handed over by a bridge session; the admin
    /// evaluator persists them ([`take_users`](Self::take_users)).
    pub fn offer_users(&self, users: Vec<PlatformUser>) {
        let mut v = self.users_inbox.lock().unwrap_or_else(|e| e.into_inner());
        v.extend(users);
        if v.len() > 10_000 {
            let drop = v.len() - 10_000;
            v.drain(..drop);
        }
    }

    pub fn take_users(&self) -> Vec<PlatformUser> {
        std::mem::take(&mut *self.users_inbox.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// One account's behaviour over the window.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountActivity {
    pub login: u64,
    pub name: String,
    pub group: String,
    pub platforms: Vec<String>,
    pub ips: Vec<String>,
    pub orders: u32,
    pub cancels: u32,
    /// Most client orders in any 60 s.
    pub max_per_min: u32,
    pub closes: u32,
    pub median_hold_s: Option<u64>,
    /// Closes held shorter than `scalper_hold_s`, percent of closes.
    pub scalp_pct: u8,
    pub connects: u32,
    pub disconnects: u32,
    pub auth_fails: u32,
    /// `scalper` | `burst` | `churn` | `brute_force` | `ip_hopping`
    pub flags: Vec<&'static str>,
    pub score: u32,
}

/// An IP address seen failing authentication or hitting the connection limit.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpActivity {
    pub ip: String,
    pub auth_fails: u32,
    pub key_fails: u32,
    pub conn_rejects: u32,
    pub accounts: Vec<u64>,
    pub flags: Vec<&'static str>,
}

#[derive(Default)]
struct Acc {
    platforms: BTreeSet<String>,
    ips: BTreeSet<String>,
    order_ts: Vec<u64>,
    cancels: u32,
    holds: Vec<u64>,
    connects: u32,
    disconnects: u32,
    auth_fails: u32,
}

/// Scores every account with activity in the last `th.window_h` hours.
/// `resolve` maps a token's external account id to the engine number.
pub fn account_activity(
    e: &Engine,
    admin: &AdminState,
    events: &[ActivityEvent],
    now_ns: u64,
    th: &BehaviorThresholds,
    resolve: &dyn Fn(&str) -> Option<u64>,
) -> (Vec<AccountActivity>, Vec<IpActivity>) {
    let window_ns = u64::from(th.window_h.max(1)) * 3_600_000_000_000;
    let from_ns = now_ns.saturating_sub(window_ns);
    let from_ms = from_ns / 1_000_000;
    let mut accs: BTreeMap<u64, Acc> = BTreeMap::new();
    let mut ips: BTreeMap<String, IpActivity> = BTreeMap::new();

    for o in e.orders() {
        if o.created_ts < from_ns || o.origin != OrderOrigin::Client {
            continue;
        }
        let a = accs.entry(o.req.account).or_default();
        a.order_ts.push(o.created_ts);
        if o.status == OrderStatus::Cancelled {
            a.cancels += 1;
        }
        if let Some(p) = serde_json::to_value(o.req.platform)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
        {
            if p != "unknown" {
                a.platforms.insert(p);
            }
        }
        if let Some(ip) = &o.req.ip {
            a.ips.insert(ip.clone());
        }
    }
    // hold time of a close = close deal − first open deal of the position
    let mut opened: BTreeMap<u64, u64> = BTreeMap::new();
    for d in e.deals() {
        if d.entry == DealEntry::In {
            opened.entry(d.position_id).or_insert(d.ts);
        } else if d.ts >= from_ns {
            if let Some(open) = opened.get(&d.position_id) {
                accs.entry(d.account)
                    .or_default()
                    .holds
                    .push(d.ts.saturating_sub(*open) / 1_000_000_000);
            }
        }
    }
    for ev in events.iter().filter(|ev| ev.ts_ms >= from_ms) {
        let mut account = ev.account;
        if account.is_none() {
            account = ev.names.iter().find_map(|n| resolve(n));
        }
        if let Some(id) = account {
            let a = accs.entry(id).or_default();
            a.platforms.insert(ev.platform.clone());
            if let Some(ip) = &ev.ip {
                a.ips.insert(ip.clone());
            }
            match ev.kind {
                ActivityKind::Connect => a.connects += 1,
                ActivityKind::Disconnect => a.disconnects += 1,
                ActivityKind::AuthFail | ActivityKind::KeyFail => a.auth_fails += 1,
                ActivityKind::ConnReject => {}
            }
        }
        if let Some(ip) = &ev.ip {
            let row = ips.entry(ip.clone()).or_insert_with(|| IpActivity {
                ip: ip.clone(),
                ..IpActivity::default()
            });
            match ev.kind {
                ActivityKind::AuthFail => row.auth_fails += 1,
                ActivityKind::KeyFail => row.key_fails += 1,
                ActivityKind::ConnReject => row.conn_rejects += 1,
                _ => {}
            }
            if let Some(id) = account {
                if !row.accounts.contains(&id) {
                    row.accounts.push(id);
                }
            }
        }
    }

    let mut rows: Vec<AccountActivity> = accs
        .into_iter()
        .map(|(login, mut a)| {
            a.order_ts.sort_unstable();
            let mut max_per_min = 0u32;
            let mut j = 0;
            for i in 0..a.order_ts.len() {
                while a.order_ts[i] - a.order_ts[j] > 60_000_000_000 {
                    j += 1;
                }
                max_per_min = max_per_min.max((i - j + 1) as u32);
            }
            a.holds.sort_unstable();
            let closes = a.holds.len() as u32;
            let median_hold_s = (closes > 0).then(|| a.holds[a.holds.len() / 2]);
            let short = a
                .holds
                .iter()
                .filter(|h| **h < u64::from(th.scalper_hold_s))
                .count();
            let scalp_pct = if closes > 0 {
                (short * 100 / a.holds.len()) as u8
            } else {
                0
            };
            let mut flags = Vec::new();
            let mut score = 0;
            if closes >= th.scalper_min_closes.max(1) && scalp_pct >= th.scalper_pct {
                flags.push("scalper");
                score += 30;
            }
            if th.burst_per_min > 0 && max_per_min >= th.burst_per_min {
                flags.push("burst");
                score += 25;
            }
            if th.churn_connects > 0 && a.connects >= th.churn_connects {
                flags.push("churn");
                score += 20;
            }
            if th.auth_fails > 0 && a.auth_fails >= th.auth_fails {
                flags.push("brute_force");
                score += 25;
            }
            if th.ip_count > 0 && a.ips.len() as u32 >= th.ip_count {
                flags.push("ip_hopping");
                score += 15;
            }
            AccountActivity {
                login,
                name: admin
                    .profiles
                    .get(&login)
                    .map(|p| p.name.clone())
                    .unwrap_or_default(),
                group: e
                    .account(login)
                    .map(|a| a.group.clone())
                    .unwrap_or_default(),
                platforms: a.platforms.into_iter().collect(),
                ips: a.ips.into_iter().collect(),
                orders: a.order_ts.len() as u32,
                cancels: a.cancels,
                max_per_min,
                closes,
                median_hold_s,
                scalp_pct,
                connects: a.connects,
                disconnects: a.disconnects,
                auth_fails: a.auth_fails,
                flags,
                score: score.min(100),
            }
        })
        .collect();
    rows.sort_by(|a, b| b.score.cmp(&a.score).then(a.login.cmp(&b.login)));
    let mut ip_rows: Vec<IpActivity> = ips
        .into_values()
        .filter(|r| r.auth_fails + r.key_fails + r.conn_rejects > 0)
        .map(|mut r| {
            if th.auth_fails > 0 && r.auth_fails + r.key_fails >= th.auth_fails {
                r.flags.push("brute_force");
            }
            if th.churn_connects > 0 && r.conn_rejects >= th.churn_connects {
                r.flags.push("flood");
            }
            r
        })
        .collect();
    ip_rows.sort_by(|a, b| {
        (b.auth_fails + b.key_fails + b.conn_rejects)
            .cmp(&(a.auth_fails + a.key_fails + a.conn_rejects))
    });
    (rows, ip_rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_is_bounded_and_filters_by_time() {
        let log = ActivityLog::default();
        for i in 0..(CAPACITY + 10) {
            log.push(ActivityEvent {
                ts_ms: i as u64,
                kind: ActivityKind::Connect,
                account: Some(1),
                names: vec![],
                platform: "terminal".into(),
                ip: None,
                detail: String::new(),
            });
        }
        assert_eq!(log.len(), CAPACITY);
        assert_eq!(log.since(CAPACITY as u64 + 5).len(), 5);
    }

    #[test]
    fn flags_follow_the_thresholds() {
        let e = Engine::new(Default::default(), Box::new(oms::NullRouter));
        let admin = AdminState::default();
        let th = BehaviorThresholds {
            auth_fails: 3,
            churn_connects: 2,
            ip_count: 2,
            ..BehaviorThresholds::default()
        };
        let now_ms = 10_000_000u64;
        let ev = |kind, ip: &str, names: Vec<&str>| ActivityEvent {
            ts_ms: now_ms - 1000,
            kind,
            account: None,
            names: names.into_iter().map(String::from).collect(),
            platform: "terminal".into(),
            ip: Some(ip.into()),
            detail: String::new(),
        };
        let events = vec![
            ev(ActivityKind::Connect, "1.1.1.1", vec!["DEMO-1"]),
            ev(ActivityKind::Connect, "2.2.2.2", vec!["DEMO-1"]),
            ev(ActivityKind::AuthFail, "9.9.9.9", vec![]),
            ev(ActivityKind::AuthFail, "9.9.9.9", vec![]),
            ev(ActivityKind::AuthFail, "9.9.9.9", vec![]),
        ];
        let resolve = |n: &str| (n == "DEMO-1").then_some(7u64);
        let (rows, ips) = account_activity(&e, &admin, &events, now_ms * 1_000_000, &th, &resolve);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].login, 7);
        assert_eq!(rows[0].connects, 2);
        assert!(rows[0].flags.contains(&"churn") && rows[0].flags.contains(&"ip_hopping"));
        assert_eq!(ips[0].ip, "9.9.9.9");
        assert_eq!(ips[0].flags, vec!["brute_force"]);
    }
}
