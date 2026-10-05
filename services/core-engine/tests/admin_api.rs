//! Admin API (`/v1/*`): auth, RBAC, balance ops with idempotency and 4-eyes,
//! audit, CRUD as journaled commands, live stream and replay determinism.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use core_engine::admin::auth::{sign_hs256, unix_now, Authenticator, Claims, Role};
use core_engine::admin::{self, store, AdminConfig};
use core_engine::{recover, spawn, EngineHandle, Settings};
use http_body_util::BodyExt;
use money::{px, qty, Currency, Money};
use oms::{Command, NewOrder};
use risk::{GroupConfig, Routing, Side, SymbolSpec};
use serde_json::{json, Value};
use std::path::Path;
use std::thread::JoinHandle;
use tower::ServiceExt;

const SECRET: &[u8] = b"test-secret-not-used-anywhere-else";
const LP_TOKEN: &str = "lp-admin-token-for-tests";

fn token(sub: &str, role: Role) -> String {
    sign_hs256(
        SECRET,
        &Claims {
            sub: sub.into(),
            name: Some(format!("{sub} name")),
            role,
            exp: unix_now() + 600,
            iat: None,
            amr: vec![],
            iss: None,
        },
    )
}

fn setup_cmds() -> Vec<Command> {
    vec![
        Command::AddSymbol(SymbolSpec::fx("EURUSD", Currency::EUR, Currency::USD, 5)),
        Command::SetGroup(GroupConfig::retail("a", Currency::USD, Routing::ABook)),
        Command::SetGroup(GroupConfig::retail("b", Currency::USD, Routing::BBook)),
        Command::Quote {
            symbol: "EURUSD".into(),
            bid: px("1.1"),
            ask: px("1.1001"),
        },
        Command::OpenAccount {
            account: 7,
            group: "a".into(),
        },
        Command::Deposit {
            account: 7,
            amount: Money::parse("10000", Currency::USD).unwrap(),
            key: "d".into(),
        },
        Command::OpenAccount {
            account: 8,
            group: "b".into(),
        },
        Command::Deposit {
            account: 8,
            amount: Money::parse("10000", Currency::USD).unwrap(),
            key: "d".into(),
        },
        Command::PlaceOrder(NewOrder::market(8, "p1", "EURUSD", Side::Buy, qty("1"))),
    ]
}

struct T {
    app: Router,
    h: EngineHandle,
    join: Option<JoinHandle<()>>,
}

impl T {
    async fn start(dir: &Path, seed: bool) -> T {
        let (h, join) = spawn(Settings::new(dir)).unwrap();
        if seed {
            for c in setup_cmds() {
                h.command(c).await.unwrap();
            }
        }
        let mut cfg = AdminConfig::new(dir);
        cfg.live_interval_ms = 50;
        cfg.cors_origins = Some(vec!["http://bo.test".into()]);
        cfg.lp_status = Some(std::sync::Arc::new(std::sync::RwLock::new(vec![
            fix_gateway::SessionStatus {
                kind: fix_gateway::SessionKind::Trading,
                sender_comp_id: "FXVPS".into(),
                target_comp_id: "LMAX".into(),
                logged_on: true,
                since_ms: 1_700_000_000_000,
                last_down_reason: None,
                rejects: 2,
            },
        ])));
        // Real fix-gateway admin endpoint with a managed config file.
        let managed =
            std::sync::Arc::new(fix_gateway::managed::Managed::open(dir.join("lp.json")).unwrap());
        let gw = fix_gateway::status_http::spawn_with_admin(
            "127.0.0.1:0",
            std::sync::Arc::default(),
            Some(fix_gateway::status_http::Admin {
                token: LP_TOKEN.into(),
                managed,
            }),
        )
        .await
        .unwrap();
        cfg.lp_admin = Some(admin::LpAdmin {
            url: format!("http://{gw}"),
            token: LP_TOKEN.into(),
        });
        let app = admin::app(h.clone(), Authenticator::hs256(SECRET), cfg).unwrap();
        T {
            app,
            h,
            join: Some(join),
        }
    }

    async fn req(
        &self,
        m: Method,
        path: &str,
        tok: Option<&str>,
        body: Option<Value>,
        hdr: &[(&str, &str)],
    ) -> (StatusCode, Value) {
        let mut b = Request::builder().method(m).uri(path);
        if let Some(t) = tok {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        for (k, v) in hdr {
            b = b.header(*k, *v);
        }
        let body = match body {
            Some(v) => {
                b = b.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let res = self
            .app
            .clone()
            .oneshot(b.body(body).unwrap())
            .await
            .unwrap();
        let st = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let v = serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into()));
        (st, v)
    }

    async fn get(&self, path: &str, tok: &str) -> (StatusCode, Value) {
        self.req(Method::GET, path, Some(tok), None, &[]).await
    }

    async fn deposit(
        &self,
        tok: &str,
        account: u64,
        kind: &str,
        amount: i64,
        key: &str,
    ) -> (StatusCode, Value) {
        self.req(
            Method::POST,
            &format!("/v1/accounts/{account}/balance-ops"),
            Some(tok),
            Some(
                json!({ "clientId": account.to_string(), "type": kind, "amount": amount,
                         "currency": "USD", "reason": "test funding", "idempotencyKey": key }),
            ),
            &[("idempotency-key", key)],
        )
        .await
    }

    fn stop(mut self) {
        self.h.shutdown();
        if let Some(j) = self.join.take() {
            j.join().unwrap();
        }
    }
}

async fn snapshot(t: &T, tok: &str) -> Vec<Value> {
    let mut out = vec![];
    for p in [
        "/v1/accounts",
        "/v1/groups",
        "/v1/symbols",
        "/v1/positions",
        "/v1/audit",
        "/v1/approvals?status=all",
        "/v1/reports/statements",
    ] {
        out.push(t.get(p, tok).await.1);
    }
    out
}

async fn next_frame(body: &mut Body) -> String {
    let f = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
        .await
        .unwrap_or_else(|_| panic!("stream frame timeout"))
        .unwrap()
        .unwrap();
    String::from_utf8_lossy(f.data_ref().unwrap()).into_owned()
}

fn admin_t() -> String {
    token("alice", Role::Admin)
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_is_required_and_validated() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let (s, _) = t.req(Method::GET, "/health", None, None, &[]).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = t.req(Method::GET, "/v1/accounts", None, None, &[]).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(v["error"]["code"], "unauthorized");
    let (s, _) = t.get("/v1/accounts", "garbage").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let wrong_key = sign_hs256(
        b"another-secret",
        &Claims {
            sub: "x".into(),
            name: None,
            role: Role::Admin,
            exp: unix_now() + 60,
            iat: None,
            amr: vec![],
            iss: None,
        },
    );
    assert_eq!(
        t.get("/v1/accounts", &wrong_key).await.0,
        StatusCode::UNAUTHORIZED
    );
    let expired = sign_hs256(
        SECRET,
        &Claims {
            sub: "x".into(),
            name: None,
            role: Role::Admin,
            exp: unix_now() - 3600,
            iat: None,
            amr: vec![],
            iss: None,
        },
    );
    assert_eq!(
        t.get("/v1/accounts", &expired).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (s, v) = t.get("/v1/me", &token("rita", Role::Risk)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["role"], "risk");
    assert!(v["permissions"]
        .as_array()
        .unwrap()
        .contains(&json!("balance.approve")));
    // dev-token endpoint is disabled without dev auth
    let (s, _) = t
        .req(
            Method::POST,
            "/auth/dev-token",
            None,
            Some(json!({"role": "admin"})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // legacy raw-engine routes need the admin role
    assert_eq!(
        t.req(Method::GET, "/accounts", None, None, &[]).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        t.get("/accounts", &token("d", Role::Dealer)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(t.get("/accounts", &admin_t()).await.0, StatusCode::OK);
    t.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn dev_token_flow() {
    let dir = tempfile::tempdir().unwrap();
    let (h, join) = spawn(Settings::new(dir.path())).unwrap();
    let auth = Authenticator::hs256(SECRET).with_dev_signer(b"dev-secret-for-test", false);
    let t = T {
        app: admin::app(h.clone(), auth, AdminConfig::new(dir.path())).unwrap(),
        h,
        join: Some(join),
    };
    let (s, v) = t
        .req(
            Method::POST,
            "/auth/dev-token",
            None,
            Some(json!({"role": "support", "name": "Sam"})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let tok = v["token"].as_str().unwrap().to_string();
    let (s, me) = t.get("/v1/me", &tok).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["role"], "support");
    assert_eq!(me["name"], "Sam");
    // the regular key keeps working next to the dev signer
    assert_eq!(t.get("/v1/me", &admin_t()).await.0, StatusCode::OK);
    let (s, _) = t
        .req(
            Method::POST,
            "/auth/dev-token",
            None,
            Some(json!({"role": "root"})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // admin is not mintable without CORE_DEV_AUTH_ADMIN=1
    let (s, _) = t
        .req(
            Method::POST,
            "/auth/dev-token",
            None,
            Some(json!({"role": "admin"})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // a caller-chosen sub is ignored: the server derives it from the role
    let (_, v) = t
        .req(
            Method::POST,
            "/auth/dev-token",
            None,
            Some(json!({"role": "risk", "sub": "someone-else"})),
            &[],
        )
        .await;
    let (_, me) = t.get("/v1/me", v["token"].as_str().unwrap()).await;
    assert_eq!(me["sub"], "dev-risk");
    t.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn rbac_matrix_is_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let group = json!({"id": "a", "name": "a", "currency": "USD", "leverage": 30, "marginMode": "retail_hedged",
        "marginCallPct": 100, "stopOutPct": 50, "commissionType": "per_lot", "commissionValue": 0,
        "markupPoints": 0, "swapMultiplier": 1, "book": "A", "symbols": ["EURUSD"]});
    let symbol = json!({"name": "EURUSD", "category": "fx", "digits": 5, "contractSize": 100000, "minLot": 0.01,
        "maxLot": 100, "lotStep": 0.01, "swapLong": 0, "swapShort": 0});
    let settings = json!({"brokerName": "x broker", "baseCurrency": "USD", "fourEyesThreshold": 1000000,
        "sessionTimeoutMin": 30, "requireMfa": true, "defaultBook": "A"});
    let user = json!({"id": "u1", "name": "Uma", "email": "uma@example.com", "role": "support", "mfa": true, "active": true, "lastLogin": null});
    // (method, path, body, permission)
    let cases: Vec<(Method, &str, Option<Value>, &str)> = vec![
        (Method::GET, "/v1/dashboard", None, "dashboard.view"),
        (Method::GET, "/v1/accounts", None, "clients.view"),
        (
            Method::PATCH,
            "/v1/accounts/7/kyc",
            Some(json!({"kyc": "approved"})),
            "clients.edit",
        ),
        (Method::PUT, "/v1/groups/a", Some(group), "groups.edit"),
        (
            Method::PUT,
            "/v1/symbols/EURUSD",
            Some(symbol),
            "symbols.edit",
        ),
        (
            Method::POST,
            "/v1/groups/a/apply-preset",
            Some(json!({"presetId": "esma-fx-major"})),
            "risk.edit",
        ),
        (
            Method::POST,
            "/v1/positions/force-close",
            Some(json!({"positionIds": ["999999"]})),
            "positions.forceClose",
        ),
        (Method::GET, "/v1/risk/margin-calls", None, "risk.view"),
        (Method::GET, "/v1/lp/sessions", None, "lp.view"),
        (Method::GET, "/v1/lp/config", None, "lp.view"),
        (Method::PUT, "/v1/lp/config", Some(json!({})), "lp.manage"),
        (Method::GET, "/v1/reports/statements", None, "reports.view"),
        (Method::GET, "/v1/audit", None, "audit.view"),
        (Method::GET, "/v1/admin-users", None, "users.view"),
        (Method::PUT, "/v1/admin-users/u1", Some(user), "users.edit"),
        (Method::GET, "/v1/settings", None, "settings.view"),
        (Method::PUT, "/v1/settings", Some(settings), "settings.edit"),
        (
            Method::POST,
            "/v1/approvals/op-404/approve",
            None,
            "balance.approve",
        ),
    ];
    for role in Role::ALL {
        let tok = token(&format!("u-{}", role.as_str()), role);
        for (m, path, body, perm) in &cases {
            let (s, v) = t.req(m.clone(), path, Some(&tok), body.clone(), &[]).await;
            let allowed = admin::auth::can(role, perm);
            if allowed {
                assert_ne!(s, StatusCode::FORBIDDEN, "{role:?} {m} {path}: {v}");
            } else {
                assert_eq!(s, StatusCode::FORBIDDEN, "{role:?} {m} {path}: {v}");
                assert_eq!(v["error"]["code"], "forbidden");
                assert_eq!(v["error"]["permission"], *perm);
            }
        }
        for kind in ["deposit", "withdraw", "credit"] {
            let key = format!("{}-{kind}", role.as_str());
            let (s, v) = t.deposit(&tok, 7, kind, 100, &key).await;
            let allowed = admin::auth::can(role, &format!("balance.{kind}"));
            assert_eq!(s == StatusCode::FORBIDDEN, !allowed, "{role:?} {kind}: {v}");
        }
    }
    // highlights of the matrix
    use admin::auth::can;
    assert!(!can(Role::Dealer, "balance.deposit"));
    assert!(!can(Role::Support, "balance.credit"));
    assert!(!can(Role::Support, "positions.forceClose"));
    assert!(can(Role::Risk, "balance.approve"));
    assert!(!can(Role::Readonly, "users.view") && can(Role::Readonly, "audit.view"));
    assert!(!can(Role::Admin, "made.up"));
    t.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn read_endpoints() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let a = admin_t();
    let (s, v) = t.get("/v1/accounts", &a).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 2);
    let (_, v) = t.get("/v1/accounts?search=8", &a).await;
    assert_eq!(v.as_array().unwrap().len(), 1);
    let (s, c) = t.get("/v1/accounts/8", &a).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(c["balance"], 1_000_000);
    assert_eq!(c["currency"], "USD");
    assert!(c["margin"].as_i64().unwrap() > 0);
    assert!(c["equity"].as_i64().unwrap() < 1_000_000); // spread
    assert_eq!(t.get("/v1/accounts/99", &a).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.get("/v1/accounts/x", &a).await.0, StatusCode::BAD_REQUEST);
    let (_, p) = t.get("/v1/accounts/8/positions", &a).await;
    assert_eq!(p.as_array().unwrap().len(), 1);
    let (_, p) = t.get("/v1/positions", &a).await;
    assert_eq!(p[0]["symbol"], "EURUSD");
    assert_eq!(p[0]["side"], "buy");
    assert_eq!(p[0]["book"], "B");
    assert_eq!(p[0]["lots"], 1.0);
    assert_eq!(p[0]["currentPrice"], 1.1);
    let (_, ex) = t.get("/v1/exposure", &a).await;
    assert_eq!(ex[0]["netLots"], 1.0);
    assert_eq!(ex[0]["bBookLots"], 1.0);
    assert_eq!(ex[0]["notional"], 10_000_000); // 100k EUR in cents
    let (s, o) = t.get("/v1/orders", &a).await;
    assert_eq!(s, StatusCode::OK);
    assert!(o.as_array().unwrap().is_empty());
    let (_, g) = t.get("/v1/groups", &a).await;
    assert_eq!(g.as_array().unwrap().len(), 2);
    assert_eq!(g[0]["book"], "A");
    let (_, sy) = t.get("/v1/symbols", &a).await;
    assert_eq!(sy[0]["name"], "EURUSD");
    assert_eq!(sy[0]["tickSize"], 0.00001);
    let (_, d) = t.get("/v1/dashboard", &a).await;
    assert_eq!(d["totalAccounts"], 2);
    assert_eq!(d["openPositions"], 1);
    assert_eq!(d["depositSeries"].as_array().unwrap().len(), 7);
    assert_eq!(
        t.get("/v1/risk/presets", &a)
            .await
            .1
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert!(t
        .get("/v1/risk/margin-calls", &a)
        .await
        .1
        .as_array()
        .unwrap()
        .is_empty());
    let (_, st) = t.get("/v1/reports/statements", &a).await;
    assert_eq!(st.as_array().unwrap().len(), 2);
    assert_eq!(t.get("/v1/reports/trades", &a).await.0, StatusCode::OK);
    let (s, v) = t.get("/v1/lp/sessions", &a).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v[0]["kind"], "TRADING");
    assert_eq!(v[0]["status"], "logged_on");
    assert_eq!(v[0]["targetCompId"], "LMAX");
    assert_eq!(v[0]["rejects24h"], 2);
    assert_eq!(t.get("/v1/lp/sessions", &a).await.0, StatusCode::OK);
    let (s, _) = t
        .req(
            Method::POST,
            "/v1/lp/sessions/x/reconnect",
            Some(&a),
            None,
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, set) = t.get("/v1/settings", &a).await;
    assert_eq!(set["fourEyesThreshold"], 1_000_000);
    t.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn balance_ops_idempotency_and_audit() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let sup = token("sam", Role::Support);
    let (s, r) = t.deposit(&sup, 7, "deposit", 50_000, "k-1").await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["status"], "applied");
    assert_eq!(r["newBalance"], 1_050_000);
    // same key: same result, no double booking
    let (s, r2) = t.deposit(&sup, 7, "deposit", 50_000, "k-1").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(r2["id"], r["id"]);
    assert_eq!(r2["replayed"], true);
    assert_eq!(t.get("/v1/accounts/7", &sup).await.1["balance"], 1_050_000);
    // same key, different request
    let (s, e) = t.deposit(&sup, 7, "deposit", 60_000, "k-1").await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(e["error"]["code"], "idempotency_conflict");
    // key required, header/body must agree
    let (s, _) = t
        .req(
            Method::POST,
            "/v1/accounts/7/balance-ops",
            Some(&sup),
            Some(
                json!({"type": "deposit", "amount": 1, "currency": "USD", "reason": "no key here"}),
            ),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = t
        .req(Method::POST, "/v1/accounts/7/balance-ops", Some(&sup),
             Some(json!({"type": "deposit", "amount": 1, "currency": "USD", "reason": "mismatch", "idempotencyKey": "a"})),
             &[("idempotency-key", "b")])
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // validation
    assert_eq!(
        t.deposit(&sup, 7, "deposit", 0, "k-0").await.0,
        StatusCode::BAD_REQUEST
    );
    let (s, e) = t
        .req(Method::POST, "/v1/accounts/7/balance-ops", Some(&sup),
             Some(json!({"type": "deposit", "amount": 5, "currency": "EUR", "reason": "wrong ccy", "idempotencyKey": "k-eur"})), &[])
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(e["error"]["code"], "currency_mismatch");
    assert_eq!(
        t.deposit(&sup, 99, "deposit", 5, "k-99").await.0,
        StatusCode::NOT_FOUND
    );
    // withdrawal beyond free margin is rejected by the engine, and can be retried
    let (s, e) = t.deposit(&sup, 8, "withdraw", 999_000, "k-w").await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["error"]["code"], "rejected");
    let (s, w) = t.deposit(&sup, 7, "withdraw", 20_000, "k-w").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(w["newBalance"], 1_030_000);
    // credit (admin only) changes credit and equity, not balance
    let (s, c) = t.deposit(&admin_t(), 7, "credit", 10_000, "k-c").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(c["newCredit"], 10_000);
    let acct = t.get("/v1/accounts/7", &sup).await.1;
    assert_eq!(acct["credit"], 10_000);
    assert_eq!(acct["balance"], 1_030_000);
    assert_eq!(acct["equity"], 1_040_000);
    // audit: newest first, with actor
    let (_, au) = t.get("/v1/audit", &sup).await;
    let au = au.as_array().unwrap();
    assert_eq!(au.len(), 3);
    assert_eq!(au[0]["action"], "balance.credit");
    assert_eq!(au[0]["actor"], "alice name");
    assert_eq!(au[0]["role"], "admin");
    assert_eq!(au[2]["action"], "balance.deposit");
    assert_eq!(au[2]["actor"], "sam name");
    assert_eq!(au[2]["target"], "#7");
    assert!(au[2]["details"].as_str().unwrap().contains("500.00 USD"));
    let (_, d) = t.get("/v1/dashboard", &admin_t()).await;
    assert_eq!(d["depositsToday"], 50_000);
    assert_eq!(d["withdrawalsToday"], 20_000);
    t.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn four_eyes_approval() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let sup = token("sam", Role::Support);
    let risk = token("rita", Role::Risk);
    let adm = admin_t();
    let (s, r) = t.deposit(&sup, 7, "deposit", 2_000_000, "big-1").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(r["status"], "pending_approval");
    let id = r["id"].as_str().unwrap().to_string();
    assert_eq!(t.get("/v1/accounts/7", &sup).await.1["balance"], 1_000_000);
    // retry with the same key is idempotent while pending
    assert_eq!(
        t.deposit(&sup, 7, "deposit", 2_000_000, "big-1").await.1["id"],
        json!(id)
    );
    let (_, q) = t.get("/v1/approvals", &risk).await;
    assert_eq!(q.as_array().unwrap().len(), 1);
    assert_eq!(q[0]["requestedBy"], "sam name");
    assert_eq!(q[0]["type"], "deposit");
    // support cannot approve; requester cannot approve own request (even admin)
    let approve = format!("/v1/approvals/{id}/approve");
    assert_eq!(
        t.req(Method::POST, &approve, Some(&sup), None, &[]).await.0,
        StatusCode::FORBIDDEN
    );
    let (s, r) = t.deposit(&adm, 7, "deposit", 1_500_000, "big-2").await;
    assert_eq!(
        (s, r["status"].clone()),
        (StatusCode::OK, json!("pending_approval"))
    );
    let own = format!("/v1/approvals/{}/approve", r["id"].as_str().unwrap());
    let (s, e) = t.req(Method::POST, &own, Some(&adm), None, &[]).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(e["error"]["code"], "four_eyes");
    // a different user with balance.approve approves
    let (s, v) = t.req(Method::POST, &approve, Some(&risk), None, &[]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "applied");
    assert_eq!(v["decidedBy"], "rita name");
    assert_eq!(t.get("/v1/accounts/7", &sup).await.1["balance"], 3_000_000);
    assert_eq!(
        t.req(Method::POST, &approve, Some(&risk), None, &[])
            .await
            .0,
        StatusCode::CONFLICT
    );
    // reject the admin's own request
    let rej = own.replace("approve", "reject");
    let (s, v) = t
        .req(
            Method::POST,
            &rej,
            Some(&risk),
            Some(json!({"reason": "no docs"})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["status"], "rejected");
    assert_eq!(v["note"], "no docs");
    assert_eq!(t.get("/v1/accounts/7", &sup).await.1["balance"], 3_000_000);
    assert!(t
        .get("/v1/approvals", &risk)
        .await
        .1
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        t.get("/v1/approvals?status=all", &risk)
            .await
            .1
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        t.get("/v1/approvals", &token("d", Role::Dealer)).await.0,
        StatusCode::FORBIDDEN
    );
    let (_, au) = t.get("/v1/audit", &adm).await;
    let actions: Vec<&str> = au
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        [
            "balance.deposit.rejected",
            "balance.deposit.approved",
            "balance.deposit.requested",
            "balance.deposit.requested"
        ]
    );
    // the threshold is a setting
    let mut set = t.get("/v1/settings", &adm).await.1;
    set["fourEyesThreshold"] = json!(10_000_000);
    assert_eq!(
        t.req(Method::PUT, "/v1/settings", Some(&adm), Some(set), &[])
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.deposit(&sup, 7, "deposit", 2_000_000, "big-3").await.1["status"],
        "applied"
    );
    t.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_symbols_positions_kyc_users() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let adm = admin_t();
    let mut g = t.get("/v1/groups", &adm).await.1[1].clone();
    assert_eq!(g["id"], "b");
    g["leverage"] = json!(100);
    g["markupPoints"] = json!(3);
    let (s, v) = t
        .req(
            Method::PUT,
            "/v1/groups/b",
            Some(&adm),
            Some(g.clone()),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["leverage"], 100);
    assert_eq!(v["markupPoints"], 3);
    g["stopOutPct"] = json!(120);
    let (s, e) = t
        .req(
            Method::PUT,
            "/v1/groups/b",
            Some(&adm),
            Some(g.clone()),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(e["error"]["message"].as_str().unwrap().contains("Stop-out"));
    g["stopOutPct"] = json!(50);
    g["currency"] = json!("EUR");
    assert_eq!(
        t.req(
            Method::PUT,
            "/v1/groups/b",
            Some(&adm),
            Some(g.clone()),
            &[]
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    g["currency"] = json!("USD");
    g["symbols"] = json!(["NOPE"]);
    assert_eq!(
        t.req(
            Method::PUT,
            "/v1/groups/b",
            Some(&adm),
            Some(g.clone()),
            &[]
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    // create
    g["symbols"] = json!(["EURUSD"]);
    let (s, v) = t
        .req(
            Method::PUT,
            "/v1/groups/vip",
            Some(&adm),
            Some(g.clone()),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["id"], "vip");
    assert_eq!(
        t.get("/v1/groups", &adm).await.1.as_array().unwrap().len(),
        3
    );
    let (s, v) = t
        .req(
            Method::POST,
            "/v1/groups/vip/apply-preset",
            Some(&adm),
            Some(json!({"presetId": "esma-fx-minor"})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["leverage"], 20);
    assert_eq!(v["esma"], "retail");
    let (s, _) = t
        .req(
            Method::POST,
            "/v1/groups/vip/apply-preset",
            Some(&adm),
            Some(json!({"presetId": "x"})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // symbols
    let mut sy = t.get("/v1/symbols", &adm).await.1[0].clone();
    sy["swapLong"] = json!(-6.2);
    let (s, v) = t
        .req(
            Method::PUT,
            "/v1/symbols/EURUSD",
            Some(&adm),
            Some(sy.clone()),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["swapLong"], -6.2);
    sy["minLot"] = json!(1000);
    assert_eq!(
        t.req(
            Method::PUT,
            "/v1/symbols/EURUSD",
            Some(&adm),
            Some(sy.clone()),
            &[]
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    sy["minLot"] = json!(0.01);
    let (s, v) = t
        .req(Method::PUT, "/v1/symbols/GBPUSD", Some(&adm), Some(sy), &[])
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["description"], "GBP/USD");
    assert_eq!(
        t.get("/v1/symbols", &adm).await.1.as_array().unwrap().len(),
        2
    );
    // force close (dealer)
    let pid = t.get("/v1/positions", &adm).await.1[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (s, v) = t
        .req(
            Method::POST,
            "/v1/positions/force-close",
            Some(&token("deniz", Role::Dealer)),
            Some(json!({"positionIds": [pid, "424242"]})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["closed"], 1);
    assert!(t
        .get("/v1/positions", &adm)
        .await
        .1
        .as_array()
        .unwrap()
        .is_empty());
    // the forced close shows up in the closed-trade report
    let (s, v) = t.get("/v1/reports/trades", &adm).await;
    assert_eq!(s, StatusCode::OK);
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{v}");
    assert_eq!(rows[0]["symbol"], "EURUSD");
    assert!(rows[0]["lots"].as_f64().unwrap() > 0.0);
    assert!(rows[0]["openPrice"].as_f64().unwrap() > 0.0);
    // kyc
    let (s, v) = t
        .req(
            Method::PATCH,
            "/v1/accounts/7/kyc",
            Some(&adm),
            Some(json!({"kyc": "approved"})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["kyc"], "approved");
    assert_eq!(
        t.req(
            Method::PATCH,
            "/v1/accounts/7/kyc",
            Some(&adm),
            Some(json!({"kyc": "maybe"})),
            &[]
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    // users
    let u = json!({"id": "u1", "name": "Uma", "email": "uma@example.com", "role": "support", "mfa": true, "active": true, "lastLogin": null});
    assert_eq!(
        t.req(
            Method::PUT,
            "/v1/admin-users/u1",
            Some(&adm),
            Some(u.clone()),
            &[]
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        t.req(Method::PUT, "/v1/admin-users/u2", Some(&adm), Some(u), &[])
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        t.get("/v1/admin-users", &adm).await.1[0]["email"],
        "uma@example.com"
    );
    let (_, au) = t.get("/v1/audit", &adm).await;
    let actions: Vec<&str> = au
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        [
            "user.update",
            "kyc.update",
            "position.forceClose",
            "symbol.update",
            "symbol.update",
            "risk.applyPreset",
            "group.update",
            "group.update"
        ]
    );
    assert_eq!(au[2]["actor"], "deniz name");
    t.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn replay_is_deterministic_with_admin_commands() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let adm = admin_t();
    let sup = token("sam", Role::Support);
    t.deposit(&sup, 7, "deposit", 12_345, "r-1").await;
    t.deposit(&adm, 7, "credit", 500, "r-2").await;
    let big = t.deposit(&sup, 8, "deposit", 5_000_000, "r-3").await.1;
    let path = format!("/v1/approvals/{}/approve", big["id"].as_str().unwrap());
    assert_eq!(
        t.req(
            Method::POST,
            &path,
            Some(&token("rita", Role::Risk)),
            None,
            &[]
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut g = t.get("/v1/groups", &adm).await.1[0].clone();
    g["leverage"] = json!(50);
    t.req(Method::PUT, "/v1/groups/a", Some(&adm), Some(g), &[])
        .await;
    let pid = t.get("/v1/positions", &adm).await.1[0]["id"].clone();
    t.req(
        Method::POST,
        "/v1/positions/force-close",
        Some(&adm),
        Some(json!({"positionIds": [pid]})),
        &[],
    )
    .await;
    t.req(
        Method::PATCH,
        "/v1/accounts/8/kyc",
        Some(&adm),
        Some(json!({"kyc": "pending"})),
        &[],
    )
    .await;

    let before = snapshot(&t, &adm).await;
    let digest =
        t.h.query(|e| Value::String(e.state_digest()))
            .await
            .unwrap();
    let live_admin = store::replay(dir.path()).unwrap();
    t.stop();

    // engine journal replay gives the same engine state (admin commands included)
    let (e, _) = recover(&Settings::new(dir.path())).unwrap();
    assert_eq!(Value::String(e.state_digest()), digest);
    e.check_invariants().unwrap();
    // admin journal replay is deterministic
    assert_eq!(store::replay(dir.path()).unwrap(), live_admin);
    assert_eq!(live_admin.audit.len(), 7, "{:#?}", live_admin.audit);

    // a restarted service serves identical state and keeps idempotency
    let t = T::start(dir.path(), false).await;
    assert_eq!(snapshot(&t, &adm).await, before);
    let (_, r) = t.deposit(&sup, 7, "deposit", 12_345, "r-1").await;
    assert_eq!(r["replayed"], true);
    assert_eq!(t.get("/v1/accounts/7", &adm).await.1, before[0][0]);
    t.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn live_stream_and_cors() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let (s, _) = t.req(Method::GET, "/v1/stream", None, None, &[]).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // the bearer token is never accepted in the URL
    let (s, _) = t
        .req(
            Method::GET,
            &format!("/v1/stream?access_token={}", token("o", Role::Readonly)),
            None,
            None,
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, tk) = t
        .req(
            Method::POST,
            "/v1/stream/ticket",
            Some(&token("o", Role::Readonly)),
            None,
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{tk}");
    let ticket = tk["ticket"].as_str().unwrap().to_string();
    let open = |q: String| {
        t.app
            .clone()
            .oneshot(Request::get(q).body(Body::empty()).unwrap())
    };
    let res = open(format!("/v1/stream?ticket={ticket}")).await.unwrap();
    // single use
    assert_eq!(
        open(format!("/v1/stream?ticket={ticket}"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));
    let mut body = res.into_body();
    let mut buf = String::new();
    buf.push_str(&next_frame(&mut body).await);
    assert!(buf.contains("event: hello"), "{buf}");
    // let the change ticker take its first sample
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    t.deposit(&admin_t(), 7, "deposit", 100, "s-1").await;
    while !buf.contains("listClients") {
        buf.push_str(&next_frame(&mut body).await);
    }
    assert!(buf.contains("event: invalidate"));
    // engine changes outside the admin API (e.g. a client order) are pushed too
    t.h.command(Command::PlaceOrder(NewOrder::market(
        7,
        "c1",
        "EURUSD",
        Side::Sell,
        qty("0.1"),
    )))
    .await
    .unwrap();
    buf.clear();
    while !buf.contains("listPositions") {
        buf.push_str(&next_frame(&mut body).await);
    }
    drop(body);

    // CORS preflight for a configured origin
    let res = t
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/v1/accounts")
                .header("origin", "http://bo.test")
                .header("access-control-request-method", "POST")
                .header(
                    "access-control-request-headers",
                    "authorization,idempotency-key",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.headers()["access-control-allow-origin"],
        "http://bo.test"
    );
    let res = t
        .app
        .clone()
        .oneshot(
            Request::get("/health")
                .header("origin", "http://evil.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(res.headers().get("access-control-allow-origin").is_none());
    t.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn seed_populates_empty_engine_once() {
    let dir = tempfile::tempdir().unwrap();
    let (h, join) = spawn(Settings::new(dir.path())).unwrap();
    assert!(admin::seed::seed_if_empty(&h).await.unwrap());
    assert!(!admin::seed::seed_if_empty(&h).await.unwrap());
    let n = h.query(|e| json!(e.accounts().count())).await.unwrap();
    assert_eq!(n, 6);
    let pos = h
        .query(|e| {
            json!(e
                .accounts()
                .map(|a| e.positions_of(a.id).len())
                .sum::<usize>())
        })
        .await
        .unwrap();
    assert_eq!(pos, 7);
    h.shutdown();
    join.join().unwrap();
}

fn lp_config(password: Option<&str>) -> Value {
    let mut c = fix_gateway::GatewayConfig::from_toml(include_str!(
        "../../fix-gateway/config/default.toml"
    ))
    .unwrap();
    c.md.addr = "fix-md.example.com:443".into();
    c.md.password = password.map(Into::into);
    c.trade.password = password.map(Into::into);
    serde_json::to_value(c).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn lp_config_through_gateway() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), false).await;
    let a = token("root", Role::Admin);
    let d = token("dealer", Role::Dealer);

    // Nothing stored yet.
    let (s, v) = t.get("/v1/lp/config", &a).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v.is_null());

    // Dealer may view but not manage.
    let (s, _) = t
        .req(
            Method::PUT,
            "/v1/lp/config",
            Some(&d),
            Some(lp_config(None)),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // Gateway-side validation surfaces as 400.
    let mut bad = lp_config(None);
    bad["md"]["addr"] = json!("no-port");
    let (s, v) = t
        .req(Method::PUT, "/v1/lp/config", Some(&a), Some(bad), &[])
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    let (s, v) = t
        .req(
            Method::PUT,
            "/v1/lp/config",
            Some(&a),
            Some(lp_config(Some("pa55word"))),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!v.to_string().contains("pa55word"));
    assert_eq!(v["md"]["password_set"], true);

    let (_, v) = t.get("/v1/lp/config", &d).await;
    assert_eq!(v["md"]["addr"], "fix-md.example.com:443");
    assert!(v["md"]["password"].is_null());

    let (_, audit) = t.get("/v1/audit", &a).await;
    let row = audit
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["action"] == "lp.config")
        .expect("audit row");
    let text = row["details"].as_str().unwrap();
    assert!(text.contains("fix-md.example.com:443") && text.contains("password changed"));
    assert!(!audit.to_string().contains("pa55word"));
    t.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn open_account_and_fund() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let a = token("root", Role::Admin);
    let d = token("dealer", Role::Dealer);
    let req = json!({"name": "Saygın Balıkel", "email": "Saygin@Example.com", "group": "a"});
    let (s, _) = t
        .req(
            Method::POST,
            "/v1/accounts",
            Some(&d),
            Some(req.clone()),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t
        .req(
            Method::POST,
            "/v1/accounts",
            Some(&a),
            Some(json!({"name": "X Y", "email": "x@y.z", "group": "nope"})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = t
        .req(Method::POST, "/v1/accounts", Some(&a), Some(req), &[])
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["login"], 100_001);
    assert_eq!(v["name"], "Saygın Balıkel");
    assert_eq!(v["email"], "saygin@example.com");
    assert_eq!(v["currency"], "USD");
    let (s, v) = t
        .req(
            Method::POST,
            "/v1/accounts/100001/balance-ops",
            Some(&a),
            Some(json!({"type": "deposit", "amount": 500_000, "currency": "USD", "reason": "demo funding", "idempotencyKey": "open-1"})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, c) = t.get("/v1/accounts/100001", &a).await;
    assert_eq!(c["balance"], 500_000);
    let (_, audit) = t.get("/v1/audit", &a).await;
    assert!(audit.to_string().contains("account.open"));
    t.stop();
}
