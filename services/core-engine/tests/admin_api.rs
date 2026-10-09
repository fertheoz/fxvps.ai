//! Admin API (`/v1/*`): auth, RBAC, balance ops with idempotency and 4-eyes,
//! audit, CRUD as journaled commands, live stream and replay determinism.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use core_engine::admin::auth::{sign_hs256, unix_now, Actor, Authenticator, Claims, Role};
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
    async fn start_with(dir: &Path, seed: bool, tune: impl FnOnce(&mut AdminConfig)) -> T {
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
                lp: "LMAX".into(),
                sender_comp_id: "FXVPS".into(),
                target_comp_id: "LMAX".into(),
                logged_on: true,
                since_ms: 1_700_000_000_000,
                last_down_reason: None,
                rejects: 2,
                in_seq: 0,
                last_msg_ms: 0,
                latency_ms: 0,
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
        tune(&mut cfg);
        let app = admin::app(h.clone(), Authenticator::hs256(SECRET), cfg).unwrap();
        T {
            app,
            h,
            join: Some(join),
        }
    }

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
        // client self-service (`/v1/client/*`): plain account numbers resolve
        cfg.names = Some(core_engine::api::AccountNames::default());
        cfg.lp_status = Some(std::sync::Arc::new(std::sync::RwLock::new(vec![
            fix_gateway::SessionStatus {
                kind: fix_gateway::SessionKind::Trading,
                lp: "LMAX".into(),
                sender_comp_id: "FXVPS".into(),
                target_comp_id: "LMAX".into(),
                logged_on: true,
                since_ms: 1_700_000_000_000,
                last_down_reason: None,
                rejects: 2,
                in_seq: 0,
                last_msg_ms: 0,
                latency_ms: 0,
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

/// Identity client token for account 7: interactive (`scope: None`) or
/// minted from an API key (`read` / `trade`, `amr: apikey`).
fn client_t(scope: Option<&str>) -> String {
    let mut c = json!({
        "sub": "client-7", "exp": unix_now() + 600, "email": "c7@example.com",
        "roles": ["client"], "accounts": ["7"], "amr": ["pwd"],
    });
    if let Some(s) = scope {
        c["scope"] = json!(s);
        c["amr"] = json!(["apikey"]);
    }
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &c,
        &jsonwebtoken::EncodingKey::from_secret(SECRET),
    )
    .unwrap()
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
        (
            Method::PATCH,
            "/v1/accounts/7/group",
            Some(json!({"group": "a"})),
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
        (
            Method::GET,
            "/v1/reports/lp-executions",
            None,
            "reports.view",
        ),
        (Method::GET, "/v1/reports/revenue", None, "reports.view"),
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

#[tokio::test]
async fn read_views_see_admin_state() {
    // regression: view_state() used to copy only credit / KYC / profiles, so
    // strategies, IB links and balance ops were invisible to read views
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let a = admin_t();
    copy_accounts(&t, &[20]).await;
    let (s, _) = t
        .req(
            Method::PUT,
            "/v1/copy/strategies/20",
            Some(&a),
            Some(json!({"name": "Alpha", "perfFeeBps": 2000, "public": true})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = t.get("/v1/copy", &a).await;
    assert_eq!(v["strategies"].as_array().unwrap().len(), 1);
    assert_eq!(v["strategies"][0]["name"], "Alpha");
    let (s, _) = t
        .req(
            Method::PATCH,
            "/v1/accounts/8/ib",
            Some(&a),
            Some(json!({"ibAccount": 7})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, r) = t.get("/v1/reports/ib", &a).await;
    assert_eq!(r["rows"][0]["ib"], 7);
    assert_eq!(r["rows"][0]["clients"], 1);
    t.stop();
}

/// Funded accounts in `demo-hedge`, a group copy trading is enabled for by
/// default (`CORE_COPY_GROUPS`).
async fn copy_accounts(t: &T, accounts: &[u64]) {
    t.h.command(Command::SetGroup(GroupConfig::retail(
        "demo-hedge",
        Currency::USD,
        Routing::BBook,
    )))
    .await
    .unwrap();
    for &account in accounts {
        t.h.command(Command::OpenAccount {
            account,
            group: "demo-hedge".into(),
        })
        .await
        .unwrap();
        t.h.command(Command::Deposit {
            account,
            amount: Money::parse("10000", Currency::USD).unwrap(),
            key: format!("d{account}"),
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn copy_gate_needs_both_sides_in_a_copy_group() {
    // regression: only the follower's group was checked, so a demo follower
    // could pay its performance fee into a live provider account
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let a = admin_t();
    copy_accounts(&t, &[20, 21]).await;
    let strategy = |account: u64| {
        let a = a.clone();
        let t = &t;
        async move {
            t.req(
                Method::PUT,
                &format!("/v1/copy/strategies/{account}"),
                Some(&a),
                Some(json!({"name": "Alpha", "perfFeeBps": 2000, "public": true})),
                &[],
            )
            .await
            .0
        }
    };
    // account 7 is in a group without copy trading
    assert_eq!(strategy(7).await, StatusCode::FORBIDDEN);
    assert_eq!(strategy(20).await, StatusCode::OK);
    let subscribe = || {
        let a = a.clone();
        let t = &t;
        async move {
            t.req(
                Method::POST,
                "/v1/copy/subscribe",
                Some(&a),
                Some(json!({"follower": 21, "provider": 20})),
                &[],
            )
            .await
        }
    };
    // the strategy account was moved out of the copy groups afterwards
    let group = |account: u64, to: &str| Command::SetAccountGroup {
        account,
        group: to.into(),
    };
    t.h.command(group(20, "b")).await.unwrap();
    let (s, v) = subscribe().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    t.h.command(group(20, "demo-hedge")).await.unwrap();
    let (s, v) = subscribe().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // while subscribed, neither side can leave its group
    for account in [20, 21] {
        let ev = t.h.command(group(account, "b")).await.unwrap();
        assert!(
            ev.iter()
                .any(|e| matches!(e, oms::Event::CommandRejected { .. })),
            "{ev:?}"
        );
    }
    t.stop();
}

#[tokio::test]
async fn ib_multi_level_rebate_and_referral_code() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let a = admin_t();
    for acc in [9u64, 10] {
        t.h.command(Command::OpenAccount {
            account: acc,
            group: "b".into(),
        })
        .await
        .unwrap();
    }
    let patch = |id: u64, body: Value| {
        let a = a.clone();
        let t = &t;
        async move {
            t.req(
                Method::PATCH,
                &format!("/v1/accounts/{id}/ib"),
                Some(&a),
                Some(body),
                &[],
            )
            .await
        }
    };
    // 8 -> sub-IB 10 -> master IB 9
    assert_eq!(patch(8, json!({"ibAccount": 10})).await.0, StatusCode::OK);
    assert_eq!(
        patch(
            10,
            json!({"ibAccount": 9, "perLotCents": 500, "code": "sub-1"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        patch(9, json!({"overridePct": 50, "code": "MASTER"}))
            .await
            .0,
        StatusCode::OK
    );
    // the same code twice is refused, a malformed one too
    assert_eq!(
        patch(9, json!({"code": "SUB-1"})).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        patch(9, json!({"code": "a b"})).await.0,
        StatusCode::BAD_REQUEST
    );
    // client 8 closes its 1 lot: the sub-IB earns the per-lot rebate
    let pid = t.h.read_sync(|e| e.positions_of(8)[0].id).unwrap();
    t.h.command(Command::ClosePosition {
        account: 8,
        position_id: pid,
        volume: None,
        client_order_id: "c".into(),
    })
    .await
    .unwrap();
    let (s, r) = t.get("/v1/reports/ib", &a).await;
    assert_eq!(s, StatusCode::OK);
    let row = |ib: u64| {
        r["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["ib"] == ib)
            .unwrap()
            .clone()
    };
    let sub = row(10);
    assert_eq!(sub["rebate"], 500);
    assert_eq!(sub["payout"], 500);
    assert_eq!(sub["code"], "SUB-1");
    assert_eq!(sub["parent"], 9);
    assert_eq!(row(9)["overridePct"], 50);
    t.stop();
}

/// API-key tokens (read or trade) reach /v1/client/* through the gateway. They
/// may read, but a leaked bot key must never file a withdrawal to the
/// attacker's address, upload KYC documents, join an IB or copy a strategy.
#[tokio::test(flavor = "multi_thread")]
async fn api_key_tokens_cannot_move_money_or_change_links() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let withdraw = json!({"account": "7", "kind": "withdraw", "method": "usdt_trc20",
                          "amount": 100_000, "details": "TAttackerAddress"});
    for scope in ["read", "trade"] {
        let k = client_t(Some(scope));
        let (s, v) = t.get("/v1/client/me", &k).await;
        assert_eq!(s, StatusCode::OK, "{scope}: {v}");
        assert_eq!(v["accounts"][0]["login"], 7);
        for (path, body) in [
            ("/v1/client/funding", withdraw.clone()),
            ("/v1/client/ib/link", json!({"account": "7", "code": "ANY"})),
            (
                "/v1/client/copy/subscribe",
                json!({"account": "7", "provider": 8}),
            ),
            (
                "/v1/client/copy/unsubscribe",
                json!({"account": "7", "provider": 8}),
            ),
        ] {
            let (s, v) = t.req(Method::POST, path, Some(&k), Some(body), &[]).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{scope} {path}: {v}");
            assert_eq!(v["error"]["code"], "api_key_forbidden", "{scope} {path}");
        }
        let (s, _) = t
            .req(
                Method::POST,
                "/v1/client/kyc/documents?account=7",
                Some(&k),
                None,
                &[("content-type", "image/png"), ("x-doc-kind", "selfie")],
            )
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{scope} kyc");
        // and never a back-office route
        assert_eq!(t.get("/v1/accounts", &k).await.0, StatusCode::UNAUTHORIZED);
    }
    let (_, me) = t.get("/v1/client/me", &client_t(None)).await;
    assert_eq!(me["funding"], json!([]), "no request was filed");

    // the interactive login of the same client passes the gate
    let c = client_t(None);
    let (s, v) = t
        .req(
            Method::POST,
            "/v1/client/funding",
            Some(&c),
            Some(withdraw),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["requestedBy"], "c7@example.com");
    // empty upload: refused for the file, not for the token
    let (s, v) = t
        .req(
            Method::POST,
            "/v1/client/kyc/documents?account=7",
            Some(&c),
            None,
            &[("content-type", "image/png"), ("x-doc-kind", "selfie")],
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, _) = t
        .req(
            Method::POST,
            "/v1/client/ib/link",
            Some(&c),
            Some(json!({"account": "7", "code": "NOPE"})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    t.stop();
}

/// Trading-client token (identity service shape): `accounts` claim, no role.
fn client_token(accounts: &[&str]) -> String {
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &json!({ "sub": "client-7", "name": "Client Seven", "accounts": accounts, "exp": unix_now() + 600 }),
        &jsonwebtoken::EncodingKey::from_secret(SECRET),
    )
    .unwrap()
}

/// Serves `body` at `http://127.0.0.1:<port>/ff.json` (stand-in for the calendar feed).
async fn feed_server(body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/ff.json", axum::routing::get(move || async move { body }));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/ff.json")
}

#[tokio::test(flavor = "multi_thread")]
async fn economic_calendar_crud_import_and_client_read() {
    const FEED: &str = r#"[
      {"title":"Non-Farm Employment Change","country":"USD","date":"2026-10-09T08:30:00-04:00","impact":"High","forecast":"140K","previous":"22K"},
      {"title":"German Industrial Production m/m","country":"EUR","date":"2026-10-08T02:00:00-04:00","impact":"Medium","forecast":"","previous":"1.3%"},
      {"title":"Bank Holiday","country":"JPY","date":"2026-10-12T00:00:00-04:00","impact":"Holiday","forecast":"","previous":""},
      {"title":"Broken row","country":"USD","date":"soon","impact":"High"}
    ]"#;
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let admin = admin_t();
    let dealer = token("dan", Role::Dealer);
    let range = "from=2026-10-05&to=2026-10-19";
    // 2026-10-08T12:30:00Z
    let claims = json!({ "time": 1_791_462_600_000u64, "currency": "usd", "title": " Initial Jobless Claims ",
                         "impact": "medium", "forecast": "225K" });
    let create = "/v1/econ-calendar";

    // console: create / validate / edit (settings.edit)
    let (s, v) = t
        .req(
            Method::POST,
            create,
            Some(&dealer),
            Some(claims.clone()),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["permission"], "settings.edit");
    let (s, created) = t
        .req(
            Method::POST,
            create,
            Some(&admin),
            Some(claims.clone()),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{created}");
    assert_eq!(created["currency"], "USD");
    assert_eq!(created["title"], "Initial Jobless Claims");
    assert_eq!(created["at"], "2026-10-08T12:30:00.000Z");
    assert_eq!(created["actual"], Value::Null);
    let id = created["id"].as_str().unwrap().to_string();
    let (s, _) = t
        .req(
            Method::POST,
            create,
            Some(&admin),
            Some(claims.clone()),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let bad = json!({ "time": 1, "currency": "US", "title": "x", "impact": "high" });
    let (s, _) = t
        .req(Method::POST, create, Some(&admin), Some(bad), &[])
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let mut edited = claims.clone();
    edited["actual"] = json!("219K");
    let (s, v) = t
        .req(
            Method::PUT,
            &format!("/v1/econ-calendar/{id}"),
            Some(&admin),
            Some(edited.clone()),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        (v["id"].as_str(), v["actual"].as_str()),
        (Some(id.as_str()), Some("219K"))
    );
    let (s, _) = t
        .req(
            Method::PUT,
            "/v1/econ-calendar/nope",
            Some(&admin),
            Some(edited),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // "import this week": fetched server side, idempotent by (title, currency, time)
    std::env::set_var("CORE_ECON_FEED_URL", feed_server(FEED).await);
    let import = "/v1/econ-calendar/import";
    let (s, v) = t.req(Method::POST, import, Some(&dealer), None, &[]).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (s, v) = t.req(Method::POST, import, Some(&admin), None, &[]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        (
            v["added"].as_u64(),
            v["updated"].as_u64(),
            v["skipped"].as_u64()
        ),
        (Some(3), Some(0), Some(1))
    );
    let (s, v) = t.req(Method::POST, import, Some(&admin), None, &[]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        (
            v["added"].as_u64(),
            v["updated"].as_u64(),
            v["unchanged"].as_u64()
        ),
        (Some(0), Some(0), Some(3))
    );
    let (s, v) = t.get(&format!("/v1/econ-calendar?{range}"), &admin).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let titles: Vec<&str> = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        [
            "German Industrial Production m/m",
            "Initial Jobless Claims",
            "Non-Farm Employment Change",
            "Bank Holiday"
        ]
    );
    let (s, _) = t.get(&format!("/v1/econ-calendar?{range}"), &dealer).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // terminal: /v1/client/calendar with a client token, filtered by currency
    let client_path = format!("/v1/client/calendar?{range}");
    let (s, _) = t.req(Method::GET, &client_path, None, None, &[]).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = t.get(&client_path, &admin).await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "a staff token has no trading accounts"
    );
    let client = client_token(&["7"]);
    let (s, v) = t
        .get(&format!("{client_path}&currency=usd,JPY"), &client)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let events = v["events"].as_array().unwrap();
    assert_eq!(events.len(), 3, "{v}");
    let nfp = &events[1];
    assert_eq!(nfp["title"], "Non-Farm Employment Change");
    assert_eq!(nfp["impact"], "high");
    assert_eq!(nfp["time"], 1_791_549_000_000u64);
    assert_eq!(nfp["forecast"], "140K");
    assert_eq!(events[2]["impact"], "low");
    let (s, _) = t
        .get("/v1/client/calendar?from=2026-10-10&to=2026-10-09", &client)
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // delete, audit trail, replay
    let del = format!("/v1/econ-calendar/{id}");
    let (s, _) = t.req(Method::DELETE, &del, Some(&admin), None, &[]).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = t.req(Method::DELETE, &del, Some(&admin), None, &[]).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, audit) = t.get("/v1/audit", &admin).await;
    let actions: Vec<&str> = audit
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["action"].as_str())
        .filter(|a| a.starts_with("calendar."))
        .collect();
    assert_eq!(
        actions,
        [
            "calendar.delete",
            "calendar.import",
            "calendar.event",
            "calendar.event"
        ]
    );
    t.stop();
    let st = store::replay(dir.path()).unwrap();
    assert_eq!(st.econ_events.len(), 3);
    assert!(st
        .econ_events
        .values()
        .all(|e| e.title != "Initial Jobless Claims"));
}

#[tokio::test(flavor = "multi_thread")]
async fn swap_free_fee_on_a_book_is_reported_as_broker_revenue() {
    // regression: the A-book swap-free fee was booked against the LP
    // counterparty and the reports showed it as LP pass-through swap
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let a = admin_t();
    let mut eu = SymbolSpec::fx("EURUSD", Currency::EUR, Currency::USD, 5);
    eu.triple_swap_day = 9; // never triple, whatever weekday the test runs on
    let mut g = t.h.read_sync(|e| e.group("a").cloned()).unwrap().unwrap();
    g.swap_multiplier_pct = 0;
    g.swap_free_fee_per_lot = 500; // $5 per lot per night
    g.swap_free_grace_days = 0;
    for c in [
        Command::AddSymbol(eu),
        Command::SetGroup(g),
        Command::SetSwapConfig(risk::SwapConfig {
            skip_weekend: false,
            ..Default::default()
        }),
        // A-book account 7: 1 lot, filled by the simulated LP
        Command::PlaceOrder(NewOrder::market(7, "sf", "EURUSD", Side::Buy, qty("1"))),
        Command::Rollover,
    ] {
        t.h.command(c).await.unwrap();
    }
    let pid = t.h.read_sync(|e| e.positions_of(7)[0].id).unwrap();
    t.h.command(Command::ClosePosition {
        account: 7,
        position_id: pid,
        volume: None,
        client_order_id: "sf-c".into(),
    })
    .await
    .unwrap();
    // revenue report: the $5 fee is broker revenue, nothing passes to the LP
    let (s, r) = t.get("/v1/reports/revenue", &a).await;
    assert_eq!(s, StatusCode::OK);
    let swaps: Vec<&Value> = r["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["kind"] == "swap")
        .collect();
    assert_eq!(swaps.len(), 1, "{r}");
    assert_eq!(swaps[0]["login"], 7);
    assert_eq!(swaps[0]["book"], "A");
    assert_eq!(swaps[0]["client"], -500);
    assert_eq!(swaps[0]["broker"], 500);
    assert_eq!(swaps[0]["lp"], 0);
    assert_eq!(r["total"]["swap"], 500);
    assert_eq!(r["last24h"]["swap"], 500);
    // dashboard (and the daily report built from it)
    let (_, d) = t.get("/v1/dashboard/series?range=24h", &a).await;
    assert_eq!(d["totals"]["swap"], 500, "{}", d["totals"]);
    // institution activity: markup + commission + the fee
    let (activity, expected) =
        t.h.read_sync(|e| {
            let base: i128 = e
                .deals()
                .iter()
                .filter(|d| d.account == 7)
                .map(|d| {
                    let pnl = if d.entry == oms::DealEntry::Out {
                        d.broker_pnl
                    } else {
                        0
                    };
                    pnl - d.commission.minor
                })
                .sum();
            (
                core_engine::admin::views::account_activity(e, 7, 0),
                base + 500,
            )
        })
        .unwrap();
    assert_eq!(activity["revenue"], expected as i64);
    t.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn tenant_hostnames_are_validated_and_drive_the_public_brand() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), false).await;
    let a = admin_t();
    let put = |body: Value| {
        let a = a.clone();
        let t = &t;
        async move {
            t.req(Method::PUT, "/v1/tenants", Some(&a), Some(body), &[])
                .await
        }
    };
    // scheme, port, path and spaces can never match `location.hostname`
    for bad in [
        "https://trade.acme.com",
        "trade.acme.com:443",
        "trade.acme.com/",
        "trade acme.com",
    ] {
        let (s, v) = put(json!([{ "id": "acme", "name": "Acme", "hostnames": [bad] }])).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }
    // one hostname cannot brand two tenants
    let (s, v) = put(json!([
        { "id": "acme", "name": "Acme", "hostnames": ["trade.shared.com"] },
        { "id": "zeta", "name": "Zeta", "hostnames": [" Trade.Shared.COM "] },
    ]))
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    // stored trimmed, lower-cased, without blanks and repeats
    let (s, v) = put(json!([{
        "id": "acme", "name": "Acme", "brandColor": "#112233",
        "hostnames": [" Trade.ACME.com ", "trade.acme.com", ""],
    }]))
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v[0]["hostnames"], json!(["trade.acme.com"]));
    // the brand lookup needs no token
    let (s, b) = t
        .req(
            Method::GET,
            "/v1/client/brand?host=TRADE.acme.com",
            None,
            None,
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["tenant"], "acme");
    assert_eq!(b["brandColor"], "#112233");
    let (s, b) = t
        .req(
            Method::GET,
            "/v1/client/brand?host=other.example",
            None,
            None,
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["tenant"], Value::Null);
    assert_eq!(b["brandColor"], "");
    t.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn statement_email_status_and_test_send() {
    let dir = tempfile::tempdir().unwrap();
    let (h, join) = spawn(Settings::new(dir.path())).unwrap();
    for c in setup_cmds() {
        h.command(c).await.unwrap();
    }
    let (mailer, stub) = admin::statement::Mailer::stub("fxvps <statements@fxvps.test>").unwrap();
    let mut cfg = AdminConfig::new(dir.path());
    cfg.mailer = Some(std::sync::Arc::new(mailer));
    let t = T {
        app: admin::app(h.clone(), Authenticator::hs256(SECRET), cfg).unwrap(),
        h,
        join: Some(join),
    };
    // an identity-style staff token: the e-mail is the only name
    let ops = sign_hs256(
        SECRET,
        &Claims {
            sub: "ops@fxvps.ai".into(),
            name: None,
            role: Role::Admin,
            exp: unix_now() + 600,
            iat: None,
            amr: vec![],
            iss: None,
        },
    );
    let (s, v) = t
        .req(
            Method::POST,
            "/v1/accounts",
            Some(&ops),
            Some(json!({"name": "Ayşe <b>Yılmaz</b>", "email": "ayse@mail.com", "group": "a"})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let login = v["login"].as_u64().unwrap();
    let (s, st) = t.get("/v1/settings/statement-email", &ops).await;
    assert_eq!(s, StatusCode::OK, "{st}");
    // the monthly run is off unless CORE_STATEMENT_EMAIL=1; the channel is there
    assert_eq!(st["enabled"], false);
    assert_eq!(st["configured"], true);
    assert_eq!(st["from"], "statements@fxvps.test");
    assert_eq!(st["recipients"], 1);
    assert!(st["run"].is_null());
    // support may not send
    let (s, _) = t
        .req(
            Method::POST,
            "/v1/settings/statement-email/test",
            Some(&token("sam", Role::Support)),
            Some(json!({})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, r) = t
        .req(
            Method::POST,
            "/v1/settings/statement-email/test",
            Some(&ops),
            Some(json!({})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["to"], "ops@fxvps.ai");
    assert_eq!(r["account"], login);
    let sent = stub.messages().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0.to()[0].to_string(), "ops@fxvps.ai");
    let raw = &sent[0].1;
    assert!(raw.contains("[TEST]"), "{raw}");
    assert!(raw.contains("multipart/alternative"));
    // a staff token without an address is refused
    let (s, _) = t
        .req(
            Method::POST,
            "/v1/settings/statement-email/test",
            Some(&admin_t()),
            Some(json!({ "account": login })),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, audit) = t.get("/v1/audit", &ops).await;
    assert!(audit.to_string().contains("statement.test"));
    t.stop();

    // without an SMTP channel the test send says so
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let (s, v) = t
        .req(
            Method::POST,
            "/v1/settings/statement-email/test",
            Some(&ops),
            Some(json!({ "account": 7 })),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "mail_not_configured");
    t.stop();
}

const DAY: u64 = 86_400_000_000_000;

fn today() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
        / DAY
}

/// UTC day number -> `YYYY-MM-DD`.
fn date(day: u64) -> String {
    admin::views::iso(day * DAY)[..10].to_string()
}

/// Engine and admin journals as a running system would have left them, with
/// the given timestamps (so deals can fall on days that are already over).
fn write_journals(
    dir: &Path,
    engine: Vec<(u64, Command)>,
    admin_cmds: Vec<(u64, store::AdminCmd)>,
) {
    use std::io::Write;
    let mut f = std::fs::File::create(dir.join("journal.jsonl")).unwrap();
    for (i, (ts, cmd)) in engine.into_iter().enumerate() {
        let env = oms::Envelope {
            seq: i as u64 + 1,
            ts,
            cmd,
        };
        writeln!(f, "{}", serde_json::to_string(&env).unwrap()).unwrap();
    }
    let mut f = std::fs::File::create(store::journal_path(dir)).unwrap();
    let actor = Actor {
        sub: "seed".into(),
        name: "seed".into(),
        role: Role::Admin,
        mfa_ok: true,
    };
    for (i, (ts, cmd)) in admin_cmds.into_iter().enumerate() {
        let r = store::AdminRecord {
            seq: i as u64 + 1,
            ts,
            actor: actor.clone(),
            cmd,
        };
        writeln!(f, "{}", serde_json::to_string(&r).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn ib_set_validates_first_and_refuses_loops_and_other_currencies() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let a = admin_t();
    t.h.command(Command::SetGroup(GroupConfig::retail(
        "e",
        Currency::EUR,
        Routing::BBook,
    )))
    .await
    .unwrap();
    for (acc, group) in [(9u64, "b"), (10, "b"), (11, "e")] {
        t.h.command(Command::OpenAccount {
            account: acc,
            group: group.into(),
        })
        .await
        .unwrap();
    }
    let patch = |id: u64, body: Value| {
        let a = a.clone();
        let t = &t;
        async move {
            t.req(
                Method::PATCH,
                &format!("/v1/accounts/{id}/ib"),
                Some(&a),
                Some(body),
                &[],
            )
            .await
        }
    };
    assert_eq!(patch(10, json!({"code": "SUB-1"})).await.0, StatusCode::OK);
    // a refused request changes nothing (the share used to be written first)
    assert_eq!(
        patch(9, json!({"sharePct": 40, "code": "SUB-1"})).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        patch(9, json!({"sharePct": 40, "code": "a b"})).await.0,
        StatusCode::BAD_REQUEST
    );
    // revenue is shared as is: the IB must use the client's currency
    assert_eq!(
        patch(11, json!({"sharePct": 40, "ibAccount": 9})).await.0,
        StatusCode::BAD_REQUEST
    );
    let (_, c) = t.get("/v1/accounts/9", &a).await;
    assert_eq!(c["ibSharePct"], 0);
    let (_, c) = t.get("/v1/accounts/11", &a).await;
    assert_eq!(c["ibSharePct"], 0);
    assert_eq!(c["ibAccount"], Value::Null);
    // 8 -> 10 -> 9: closing the chain back on itself is refused
    assert_eq!(patch(8, json!({"ibAccount": 10})).await.0, StatusCode::OK);
    assert_eq!(patch(10, json!({"ibAccount": 9})).await.0, StatusCode::OK);
    assert_eq!(
        patch(9, json!({"ibAccount": 8})).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        patch(9, json!({"ibAccount": 10})).await.0,
        StatusCode::BAD_REQUEST
    );
    let (_, c) = t.get("/v1/accounts/9", &a).await;
    assert_eq!(c["ibAccount"], Value::Null);
    t.stop();
}

#[tokio::test]
async fn ib_referral_needs_a_fresh_account_and_no_loop() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), true).await;
    let a = admin_t();
    t.h.command(Command::SetGroup(GroupConfig::retail(
        "e",
        Currency::EUR,
        Routing::BBook,
    )))
    .await
    .unwrap();
    for (acc, group) in [(9u64, "b"), (10, "b"), (11, "e")] {
        t.h.command(Command::OpenAccount {
            account: acc,
            group: group.into(),
        })
        .await
        .unwrap();
    }
    for (acc, code) in [(9u64, "NINE"), (10, "TEN"), (11, "EURO")] {
        let (s, _) = t
            .req(
                Method::PATCH,
                &format!("/v1/accounts/{acc}/ib"),
                Some(&a),
                Some(json!({ "code": code })),
                &[],
            )
            .await;
        assert_eq!(s, StatusCode::OK);
    }
    let link = |acc: &'static str, code: &'static str| {
        let t = &t;
        let tok = client_token(&[acc]);
        async move {
            t.req(
                Method::POST,
                "/v1/client/ib/link",
                Some(&tok),
                Some(json!({ "account": acc, "code": code })),
                &[],
            )
            .await
        }
    };
    // 8 already traded (setup): an existing client is not attached afterwards
    let (s, v) = link("8", "NINE").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "has_trades");
    // another currency, own code
    assert_eq!(link("7", "EURO").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(link("9", "NINE").await.0, StatusCode::BAD_REQUEST);
    let (s, v) = link("7", "NINE").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true);
    assert_eq!(link("7", "TEN").await.1["already"], true);
    // 9 joins 10; 10 joining 9 would close a loop
    assert_eq!(link("9", "TEN").await.0, StatusCode::OK);
    assert_eq!(link("10", "NINE").await.0, StatusCode::BAD_REQUEST);
    let (_, c) = t.get("/v1/accounts/10", &a).await;
    assert_eq!(c["ibAccount"], Value::Null);
    // dashboard: a non-IB gets an empty list straight away, the IB its clients
    let (s, v) = t.get("/v1/client/ib", &client_token(&["7"])).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["ibs"], json!([]));
    let (_, v) = t.get("/v1/client/ib", &client_token(&["9"])).await;
    assert_eq!(v["ibs"][0]["ib"], 9);
    assert_eq!(v["ibs"][0]["clients"], 1);
    t.stop();
}

#[tokio::test]
async fn ib_link_history_override_loop_and_payouts() {
    let dir = tempfile::tempdir().unwrap();
    let today = today();
    // deals of 8 and 10 three days ago, 1 lot each at 3.50 USD commission
    let old = (today - 3) * DAY + 12 * 3_600_000_000_000;
    let mut eu = SymbolSpec::fx("EURUSD", Currency::EUR, Currency::USD, 5);
    eu.commission_per_lot = Money::parse("3.5", Currency::USD).unwrap();
    let usd = |v: &str| Money::parse(v, Currency::USD).unwrap();
    let mut engine = vec![
        Command::AddSymbol(eu),
        Command::SetGroup(GroupConfig::retail("b", Currency::USD, Routing::BBook)),
        Command::Quote {
            symbol: "EURUSD".into(),
            bid: px("1.1"),
            ask: px("1.1001"),
        },
    ];
    for acc in [8u64, 9, 10, 11, 12] {
        engine.push(Command::OpenAccount {
            account: acc,
            group: "b".into(),
        });
    }
    for acc in [8u64, 10] {
        engine.push(Command::Deposit {
            account: acc,
            amount: usd("10000"),
            key: format!("d{acc}"),
        });
        engine.push(Command::PlaceOrder(NewOrder::market(
            acc,
            "p",
            "EURUSD",
            Side::Buy,
            qty("1"),
        )));
    }
    let linked = |account: u64, ib: u64| store::AdminCmd::IbLinked {
        account,
        ib: Some(ib),
        dated: false,
    };
    let plan = |account: u64, override_pct: u8| store::AdminCmd::IbPlanSet {
        account,
        plan: store::IbPlan {
            override_pct,
            ..Default::default()
        },
    };
    let later = old + 3_600_000_000_000;
    write_journals(
        dir.path(),
        engine.into_iter().map(|c| (old, c)).collect(),
        vec![
            (
                later,
                store::AdminCmd::SettingsSaved {
                    settings: store::SettingsRec {
                        four_eyes_threshold: 100,
                        ..Default::default()
                    },
                },
            ),
            // links from before link history: they count from the start, even
            // though this record is younger than the deal
            (later, linked(8, 9)),
            (
                later,
                store::AdminCmd::IbShareSet {
                    account: 9,
                    pct: 50,
                },
            ),
            // 9 -> 11 -> 12 -> 11: a loop from old data
            (later, linked(9, 11)),
            (later, linked(11, 12)),
            (later, linked(12, 11)),
            (later, plan(11, 10)),
            (later, plan(12, 10)),
        ],
    );
    let t = T::start(dir.path(), false).await;
    let a = admin_t();
    // 10 joins 9 now: its deal from three days ago is not 9's
    let (s, v) = t
        .req(
            Method::PATCH,
            "/v1/accounts/10/ib",
            Some(&a),
            Some(json!({"ibAccount": 9})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, r) = t.get("/v1/reports/ib", &a).await;
    let row = |ib: u64| {
        r["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["ib"] == ib)
            .unwrap()
            .clone()
    };
    assert_eq!(row(9)["clients"], 2);
    assert_eq!(row(9)["deals"], 1);
    assert_eq!(row(9)["commission"], 350);
    assert_eq!(row(9)["payout"], 175);
    // each IB above is paid once per deal (11 used to be paid twice)
    assert_eq!(row(11)["override"], 35);
    assert_eq!(row(12)["override"], 35);

    let yesterday = date(today - 1);
    let preview = |ib: u64, to: String| {
        let a = a.clone();
        let t = &t;
        async move {
            t.get(&format!("/v1/accounts/{ib}/ib/payout?to={to}"), &a)
                .await
        }
    };
    let pay = |ib: u64, to: String, tok: String| {
        let t = &t;
        async move {
            t.req(
                Method::POST,
                &format!("/v1/accounts/{ib}/ib/payout"),
                Some(&tok),
                Some(json!({ "to": to })),
                &[],
            )
            .await
        }
    };
    // only a day that is over can be paid
    assert_eq!(
        pay(9, date(today), a.clone()).await.0,
        StatusCode::BAD_REQUEST
    );
    let (s, p) = preview(9, yesterday.clone()).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["amount"], 175);
    assert_eq!(p["from"], Value::Null);
    assert_eq!(p["to"], yesterday.as_str());
    assert_eq!(p["currency"], "USD");
    // above the four-eyes threshold: queued, nothing paid through yet
    let (s, v) = pay(9, yesterday.clone(), a.clone()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["op"]["status"], "pending_approval");
    let first = v["op"]["id"].as_str().unwrap().to_string();
    let (_, r) = t.get("/v1/reports/ib", &a).await;
    let r9 = r["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["ib"] == 9)
        .unwrap()
        .clone();
    assert_eq!(r9["payoutPending"], first.as_str());
    assert_eq!(r9["paidThrough"], Value::Null);
    // no second request while one waits
    let (s, v) = pay(9, yesterday.clone(), a.clone()).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    // rejected: the period stays open and can be requested again
    let (s, _) = t
        .req(
            Method::POST,
            &format!("/v1/approvals/{first}/reject"),
            Some(&a),
            Some(json!({"reason": "check"})),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, p) = preview(9, yesterday.clone()).await;
    assert_eq!(p["pending"], Value::Null);
    assert_eq!(p["amount"], 175);
    let (s, v) = pay(9, yesterday.clone(), a.clone()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["op"]["status"], "pending_approval");
    assert_eq!(v["op"]["replayed"], false);
    let second = v["op"]["id"].as_str().unwrap().to_string();
    assert_ne!(first, second);
    // approved by a second user: paid, and paid through yesterday
    let (s, v) = t
        .req(
            Method::POST,
            &format!("/v1/approvals/{second}/approve"),
            Some(&token("bob", Role::Admin)),
            None,
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, c) = t.get("/v1/accounts/9", &a).await;
    assert_eq!(c["balance"], 175);
    let (_, p) = preview(9, yesterday.clone()).await;
    assert_eq!(p["paidThrough"], yesterday.as_str());
    assert_eq!(p["amount"], 0);
    let (s, v) = pay(9, yesterday.clone(), a.clone()).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "already_paid");
    // below the threshold: applied at once
    let (s, v) = pay(11, yesterday.clone(), a.clone()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["op"]["status"], "applied");
    assert_eq!(v["amount"], 35);
    let (_, c) = t.get("/v1/accounts/11", &a).await;
    assert_eq!(c["balance"], 35);
    // replaying the admin journal gives the same paid-through marks
    let st = store::replay(dir.path()).unwrap();
    assert_eq!(st.ib_paid_to.get(&9), Some(&today));
    assert_eq!(st.ib_paid_to.get(&11), Some(&today));
    t.stop();
}

/// Stand-in for hazine's paylink API: a link asks for the amount plus a
/// one-micro uniqueness tag; `pay` binds a transfer to it the way hazine's
/// checkPayment does (any amount the test chooses).
#[derive(Clone, Default)]
struct FakeHazine(std::sync::Arc<std::sync::Mutex<FakeHazineState>>);

#[derive(Default)]
struct FakeHazineState {
    down: bool,
    /// create calls, failed ones included
    calls: u32,
    /// link id -> (invoice amount, (tx hash, receivedAmount) once paid)
    links: std::collections::BTreeMap<String, (String, Option<(String, String)>)>,
}

impl FakeHazine {
    async fn serve(&self) -> String {
        type Reply = (StatusCode, axum::Json<Value>);
        fn authorized(h: &axum::http::HeaderMap) -> bool {
            h.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer hazine-test-key")
        }
        async fn create(
            axum::extract::State(hz): axum::extract::State<FakeHazine>,
            headers: axum::http::HeaderMap,
            axum::Json(b): axum::Json<Value>,
        ) -> Reply {
            let mut s = hz.0.lock().unwrap();
            s.calls += 1;
            if !authorized(&headers) {
                return (
                    StatusCode::UNAUTHORIZED,
                    axum::Json(json!({"message": "Invalid API key"})),
                );
            }
            if s.down {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(json!({"message": "maintenance"})),
                );
            }
            let id = format!("00000000-0000-4000-8000-{:012}", s.links.len() + 1);
            // "150.00" -> "150.000001"
            let amount = format!("{}0001", b["amount"].as_str().unwrap_or("0"));
            s.links.insert(id.clone(), (amount.clone(), None));
            (
                StatusCode::CREATED,
                axum::Json(
                    json!({"id": id, "amount": amount, "status": "pending", "receivedAmount": "0", "txHash": ""}),
                ),
            )
        }
        async fn check(
            axum::extract::State(hz): axum::extract::State<FakeHazine>,
            headers: axum::http::HeaderMap,
            axum::extract::Path(id): axum::extract::Path<String>,
        ) -> Reply {
            if !authorized(&headers) {
                return (
                    StatusCode::UNAUTHORIZED,
                    axum::Json(json!({"message": "Invalid API key"})),
                );
            }
            let s = hz.0.lock().unwrap();
            let Some((amount, paid)) = s.links.get(&id) else {
                return (
                    StatusCode::NOT_FOUND,
                    axum::Json(json!({"message": "Payment request not found."})),
                );
            };
            let (status, tx, got) = match paid {
                Some((tx, got)) => ("paid", tx.as_str(), got.as_str()),
                None => ("pending", "", "0"),
            };
            (
                StatusCode::CREATED,
                axum::Json(
                    json!({"id": id, "amount": amount, "status": status, "txHash": tx, "receivedAmount": got}),
                ),
            )
        }
        let app = axum::Router::new()
            .route("/api/paylink-api", axum::routing::post(create))
            .route("/api/paylink-api/{id}/check", axum::routing::post(check))
            .with_state(self.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn set_down(&self, down: bool) {
        self.0.lock().unwrap().down = down;
    }

    fn calls(&self) -> u32 {
        self.0.lock().unwrap().calls
    }

    fn pay(&self, id: &str, tx: &str, received: &str) {
        let mut s = self.0.lock().unwrap();
        s.links.get_mut(id).unwrap().1 = Some((tx.into(), received.into()));
    }
}

async fn usdt_deposit(t: &T, tok: &str, account: &str, amount: i64) -> (StatusCode, Value) {
    let body =
        json!({"account": account, "kind": "deposit", "method": "usdt_trc20", "amount": amount});
    t.req(
        Method::POST,
        "/v1/client/funding",
        Some(tok),
        Some(body),
        &[],
    )
    .await
}

async fn decide_funding(t: &T, tok: &str, id: &str, body: Value) -> (StatusCode, Value) {
    let path = format!("/v1/funding/{id}/decide");
    t.req(Method::POST, &path, Some(tok), Some(body), &[]).await
}

/// Ids in the back office's default (open) funding list.
async fn open_funding(t: &T, tok: &str) -> Vec<String> {
    let v = t.get("/v1/funding?status=open", tok).await.1;
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect()
}

/// The back-office row of a funding request, once `ready` holds (the hazine
/// watcher polls in the background).
async fn funding_row(t: &T, tok: &str, id: &str, ready: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..200 {
        let rows = t.get("/v1/funding?status=all", tok).await.1;
        if let Some(r) = rows.as_array().unwrap().iter().find(|r| r["id"] == id) {
            if ready(r) {
                return r.clone();
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("funding request {id} never got ready");
}

/// USDT deposits through hazine: the link is opened before anything is
/// journaled, only an exact payment lets one approval credit, and a payment
/// that arrives after the decision is still recorded and flagged.
#[tokio::test(flavor = "multi_thread")]
async fn usdt_card_deposits_follow_hazine() {
    let dir = tempfile::tempdir().unwrap();
    let hz = FakeHazine::default();
    let url = hz.serve().await;
    let tune = |cfg: &mut AdminConfig| {
        let mut h = admin::usdt_watch::Hazine::new(&url, "hazine-test-key");
        h.poll = std::time::Duration::from_millis(20);
        cfg.hazine = Some(h);
        cfg.names = Some(core_engine::api::AccountNames::default());
    };
    let t = T::start_with(dir.path(), true, tune).await;
    // a EUR account next to the USD ones
    t.h.command(Command::SetGroup(GroupConfig::retail(
        "e",
        Currency::EUR,
        Routing::BBook,
    )))
    .await
    .unwrap();
    t.h.command(Command::OpenAccount {
        account: 9,
        group: "e".into(),
    })
    .await
    .unwrap();
    let a = admin_t();
    let c = client_token_as("cli", &["7", "9"]);
    let (s, me) = t.get("/v1/client/me", &c).await;
    assert_eq!(s, StatusCode::OK, "{me}");
    assert_eq!(me["cryptoCard"], true);
    // hazine down: an error for the client and nothing journaled
    hz.set_down(true);
    let (s, e) = usdt_deposit(&t, &c, "7", 1_500_000).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{e}");
    assert_eq!(t.get("/v1/funding?status=all", &a).await.1, json!([]));
    hz.set_down(false);
    // non-USD account: refused before hazine is even asked
    let calls = hz.calls();
    let (s, _) = usdt_deposit(&t, &c, "9", 10_000).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(hz.calls(), calls);
    assert_eq!(t.get("/v1/funding?status=all", &a).await.1, json!([]));
    let audit = t.get("/v1/audit", &a).await.1.to_string();
    assert!(!audit.contains("funding.deposit"), "{audit}");

    // 1) overpaid (hazine binds 1x..2x): recorded, but four-eyes stays
    let (s, fr1) = usdt_deposit(&t, &c, "7", 1_500_000).await;
    assert_eq!(s, StatusCode::OK, "{fr1}");
    let pay1 = fr1["payId"].as_str().unwrap().to_string();
    assert_eq!(fr1["payUrl"], format!("{url}/pay?id={pay1}"));
    assert_eq!(fr1["expectedMicro"], 15_000_000_001i64);
    assert_eq!(fr1["details"], "");
    let id1 = fr1["id"].as_str().unwrap().to_string();
    hz.pay(&pay1, "tx1", "15000.5");
    let r = funding_row(&t, &a, &id1, |r| r["txHash"].is_string()).await;
    assert_eq!(r["txHash"], "tx1");
    assert_eq!(r["receivedMicro"], 15_000_500_000i64);
    assert_eq!(r["paidAfterDecision"], false);
    let (s, v) = decide_funding(&t, &a, &id1, json!({"decision": "approve"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let q = t.get("/v1/approvals", &a).await.1;
    assert_eq!(q.as_array().unwrap().len(), 1, "{q}");
    let reason = q[0]["reason"].as_str().unwrap_or_default().to_string();
    assert!(reason.contains("received 15000.500000 USDT"), "{reason}");
    assert_eq!(t.get("/v1/accounts/7", &a).await.1["balance"], 1_000_000);

    // 2) exactly the tagged amount: one approval credits
    let (s, fr2) = usdt_deposit(&t, &c, "7", 1_500_000).await;
    assert_eq!(s, StatusCode::OK, "{fr2}");
    let id2 = fr2["id"].as_str().unwrap().to_string();
    hz.pay(fr2["payId"].as_str().unwrap(), "tx2", "15000.000001");
    funding_row(&t, &a, &id2, |r| r["txHash"].is_string()).await;
    let (s, v) = decide_funding(&t, &a, &id2, json!({"decision": "approve"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(t.get("/v1/accounts/7", &a).await.1["balance"], 2_500_000);
    let q = t.get("/v1/approvals", &a).await.1;
    assert_eq!(q.as_array().unwrap().len(), 1, "{q}");

    // 3) rejected, then paid on the card that stayed open: still recorded,
    // flagged and kept in the open list until someone handles it
    let (s, fr3) = usdt_deposit(&t, &c, "7", 20_000).await;
    assert_eq!(s, StatusCode::OK, "{fr3}");
    let id3 = fr3["id"].as_str().unwrap().to_string();
    let rej = json!({"decision": "reject", "note": "duplicate"});
    assert_eq!(decide_funding(&t, &a, &id3, rej).await.0, StatusCode::OK);
    hz.pay(fr3["payId"].as_str().unwrap(), "tx3", "200.000001");
    let r = funding_row(&t, &a, &id3, |r| r["txHash"].is_string()).await;
    assert_eq!(r["status"], "rejected");
    assert_eq!(r["paidAfterDecision"], true);
    // decided requests leave the open list; a late payment brings one back
    assert_eq!(open_funding(&t, &a).await, vec![id3.clone()]);
    let audit = t.get("/v1/audit", &a).await.1.to_string();
    assert!(audit.contains("after the request was Rejected"), "{audit}");
    let (s, _) = decide_funding(&t, &a, &id3, json!({"decision": "handled"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a note is required");
    let done = json!({"decision": "handled", "note": "refunded to sender"});
    let (s, v) = decide_funding(&t, &a, &id3, done).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["lateHandledBy"], "alice name");
    assert!(open_funding(&t, &a).await.is_empty());
    let again = json!({"decision": "handled", "note": "again"});
    assert_eq!(
        decide_funding(&t, &a, &id3, again).await.0,
        StatusCode::BAD_REQUEST
    );

    // 4) approved before the money came: the payment is still recorded
    let (s, fr4) = usdt_deposit(&t, &c, "7", 30_000).await;
    assert_eq!(s, StatusCode::OK, "{fr4}");
    let id4 = fr4["id"].as_str().unwrap().to_string();
    let ok = json!({"decision": "approve"});
    assert_eq!(decide_funding(&t, &a, &id4, ok).await.0, StatusCode::OK);
    hz.pay(fr4["payId"].as_str().unwrap(), "tx4", "300.000001");
    let r = funding_row(&t, &a, &id4, |r| r["txHash"].is_string()).await;
    assert_eq!(r["status"], "approved");
    assert_eq!(r["paidAfterDecision"], true);
    assert_eq!(open_funding(&t, &a).await, vec![id4.clone()]);

    // the journal replays to the same requests
    let before = t.get("/v1/funding?status=all", &a).await.1;
    t.stop();
    let t = T::start_with(dir.path(), false, tune).await;
    assert_eq!(t.get("/v1/funding?status=all", &a).await.1, before);
    t.stop();
}

/// Client token for a given subject (USDT tests).
fn client_token_as(sub: &str, accounts: &[&str]) -> String {
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &json!({ "sub": sub, "name": format!("{sub} name"), "exp": unix_now() + 600, "accounts": accounts }),
        &jsonwebtoken::EncodingKey::from_secret(SECRET),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn tenant_scoped_staff_cannot_touch_tenants_or_other_users() {
    let dir = tempfile::tempdir().unwrap();
    let t = T::start(dir.path(), false).await;
    let a = admin_t();
    let (s, _) = t
        .req(
            Method::PUT,
            "/v1/tenants",
            Some(&a),
            Some(json!([
                { "id": "acme", "name": "Acme", "groups": ["a"] },
                { "id": "zeta", "name": "Zeta", "groups": ["b"] },
            ])),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let user = |id: &str, tenant: Value| {
        json!({ "id": id, "name": format!("{id} name"), "email": format!("{id}@x.test"),
                "role": "admin", "mfa": false, "active": true, "lastLogin": null, "tenant": tenant })
    };
    for (id, tn) in [
        ("tadmin", json!("acme")),
        ("zuser", json!("zeta")),
        ("root", Value::Null),
    ] {
        let (s, v) = t
            .req(
                Method::PUT,
                &format!("/v1/admin-users/{id}"),
                Some(&a),
                Some(user(id, tn)),
                &[],
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    // a white-label admin (tenant acme) with users.edit
    let w = token("tadmin", Role::Admin);
    let (s, _) = t.get("/v1/tenants", &w).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t
        .req(
            Method::PUT,
            "/v1/tenants",
            Some(&w),
            Some(json!([{ "id": "acme", "name": "Acme", "groups": ["a", "b"] }])),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // cannot drop its own scope, nor edit another tenant's user
    let (s, _) = t
        .req(
            Method::PUT,
            "/v1/admin-users/tadmin",
            Some(&w),
            Some(user("tadmin", Value::Null)),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t
        .req(
            Method::PUT,
            "/v1/admin-users/zuser",
            Some(&w),
            Some(user("zuser", json!("acme"))),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // may add staff to its own tenant, and lists only those
    let (s, _) = t
        .req(
            Method::PUT,
            "/v1/admin-users/t2",
            Some(&w),
            Some(user("t2", json!("acme"))),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = t.get("/v1/admin-users", &w).await;
    let ids: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["id"].as_str())
        .collect();
    assert_eq!(ids, ["t2", "tadmin"]);
    // platform staff still see everything
    let (_, v) = t.get("/v1/admin-users", &a).await;
    assert_eq!(v.as_array().unwrap().len(), 4);
    t.stop();
}
