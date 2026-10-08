//! User REST API (`/api/v1`) against an in-process server: bearer auth, the
//! read-only scope, account claims, rate limits, and (with the demo core)
//! orders, idempotent client order ids, positions and deal history.

use std::sync::Arc;
use std::time::Duration;

use client_gateway::auth::{issue_hs256, Authenticator, Claims};
use client_gateway::demo::{Demo, DemoOptions};
use client_gateway::{ClientGatewayConfig, Hub};
use domain::Fixed;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};

const KEY: &[u8] = b"rest-test-key";

async fn serve(hub: Arc<Hub>) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(client_gateway::serve(hub, l));
    format!("http://{addr}/api/v1")
}

/// API-key style token (`scope` = read / trade).
fn key_token(accounts: &[&str], scope: &str) -> String {
    encode(
        &Header::new(Algorithm::HS256),
        &Claims {
            sub: "algo-user".into(),
            exp: domain::now_ns() / 1_000_000_000 + 300,
            accounts: accounts.iter().map(|s| s.to_string()).collect(),
            roles: vec!["client".into()],
            amr: vec!["apikey".into()],
            scope: Some(scope.into()),
            sid: Some(format!("apikey:{scope}")),
        },
        &EncodingKey::from_secret(KEY),
    )
    .unwrap()
}

struct Api {
    base: String,
    http: reqwest::Client,
}

impl Api {
    fn new(base: String) -> Api {
        Api {
            base,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap(),
        }
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut r = self.http.request(method, format!("{}{path}", self.base));
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await.unwrap();
        let status = resp.status();
        let text = resp.text().await.unwrap();
        let v = serde_json::from_str(&text).unwrap_or_else(|_| panic!("not JSON: {text:?}"));
        (status, v)
    }

    async fn get(&self, path: &str, token: &str) -> (StatusCode, Value) {
        self.call(Method::GET, path, Some(token), None).await
    }

    async fn post(&self, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
        self.call(Method::POST, path, Some(token), Some(body)).await
    }
}

fn code(v: &Value) -> &str {
    v["error"]["code"].as_str().unwrap_or_default()
}

fn fx(v: &Value) -> Fixed {
    v.as_str()
        .unwrap_or_else(|| panic!("decimal string expected: {v}"))
        .parse()
        .unwrap()
}

fn off(p: Fixed, d: &str) -> String {
    (p + d.parse::<Fixed>().unwrap()).to_string()
}

#[tokio::test]
async fn auth_scope_and_account_gates() {
    let cfg = ClientGatewayConfig {
        orders_per_second: 1,
        order_burst: 2,
        ..Default::default()
    };
    let hub = Hub::new(
        cfg,
        Authenticator::hs256(KEY),
        vec!["EUR/USD".to_string()],
        None,
    );
    let api = Api::new(serve(hub.clone()).await);

    // --- authentication is required everywhere ---
    let resp = api
        .http
        .get(format!("{}/accounts", api.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        resp.headers().get("www-authenticate").unwrap(),
        "Bearer",
        "RFC 6750 challenge"
    );
    let (s, v) = api.call(Method::GET, "/symbols", None, None).await;
    assert_eq!((s, code(&v)), (StatusCode::UNAUTHORIZED, "unauthenticated"));
    let (s, v) = api
        .call(Method::POST, "/orders", None, Some(json!({})))
        .await;
    assert_eq!((s, code(&v)), (StatusCode::UNAUTHORIZED, "unauthenticated"));
    let (s, v) = api.get("/symbols", "not.a.jwt").await;
    assert_eq!((s, code(&v)), (StatusCode::UNAUTHORIZED, "unauthenticated"));
    let forged = issue_hs256(b"some-other-key", "u", &["A1"], 60);
    let (s, _) = api.get("/symbols", &forged).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(hub.metrics.auth_failures.get() >= 4);

    // --- a valid token reads ---
    let trade = issue_hs256(KEY, "u", &["A1"], 60);
    let (s, v) = api.get("/symbols", &trade).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["symbols"][0]["symbol"], "EURUSD");
    assert_eq!(v["symbols"][0]["contract_size"], "100000");
    let (s, v) = api.get("/nope", &trade).await;
    assert_eq!((s, code(&v)), (StatusCode::NOT_FOUND, "not_found"));
    let (s, v) = api
        .call(Method::PUT, "/orders", Some(&trade), Some(json!({})))
        .await;
    assert_eq!(
        (s, code(&v)),
        (StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed")
    );

    let order = |account: &str| {
        json!({
            "account": account, "symbol": "EURUSD", "side": "buy",
            "type": "market", "qty": "100000", "client_order_id": "c1",
        })
    };

    // --- read-only API key: reads yes, every command 403 ---
    let read = key_token(&["A1"], "read");
    let (s, _) = api.get("/symbols", &read).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = api.post("/orders", &read, order("A1")).await;
    assert_eq!((s, code(&v)), (StatusCode::FORBIDDEN, "forbidden"));
    assert_eq!(v["error"]["message"], "read-only API key");
    for (m, path, body) in [
        (Method::DELETE, "/orders/c1?account=A1", None),
        (
            Method::PATCH,
            "/orders/c1",
            Some(json!({"limit_price": "1.1"})),
        ),
        (Method::PATCH, "/positions/7", Some(json!({"sl": "1.0"}))),
        (Method::POST, "/positions/7/close", Some(json!({}))),
    ] {
        let (s, v) = api.call(m.clone(), path, Some(&read), body).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{m} {path}: {v}");
        assert_eq!(v["error"]["message"], "read-only API key", "{m} {path}");
    }

    // --- another account: forbidden for reads and commands alike ---
    let (s, v) = api.post("/orders", &trade, order("B9")).await;
    assert_eq!((s, code(&v)), (StatusCode::FORBIDDEN, "forbidden"));
    assert_eq!(v["error"]["message"], "account not authorized");
    for path in [
        "/accounts/B9",
        "/positions?account=B9",
        "/orders?account=B9",
        "/deals?account=B9",
        "/quotes/EURUSD?account=B9",
    ] {
        let (s, v) = api.get(path, &trade).await;
        assert_eq!(
            (s, code(&v)),
            (StatusCode::FORBIDDEN, "forbidden"),
            "{path}"
        );
    }
    for (m, path) in [
        (Method::DELETE, "/orders/c1?account=B9"),
        (Method::POST, "/positions/7/close?account=B9"),
    ] {
        let (s, _) = api.call(m.clone(), path, Some(&trade), None).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{m} {path}");
    }

    // --- bad input is a JSON 400 ---
    let resp = api
        .http
        .post(format!("{}/orders", api.base))
        .bearer_auth(&trade)
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(code(&v), "bad_request");
    let (s, v) = api
        .post(
            "/orders",
            &trade,
            json!({"symbol": "EURUSD", "side": "up", "qty": "1"}),
        )
        .await;
    assert_eq!(
        (s, code(&v)),
        (StatusCode::BAD_REQUEST, "bad_request"),
        "{v}"
    );
    let two = issue_hs256(KEY, "u", &["A1", "A2"], 60);
    let (s, v) = api.get("/positions", &two).await;
    assert_eq!(
        (s, code(&v)),
        (StatusCode::BAD_REQUEST, "bad_request"),
        "{v}"
    );

    // --- per-account order rate limit, same bucket as the WebSocket ---
    // Without a core the gate passes and the hub answers 503; the third
    // command within the burst of 2 is refused before reaching the hub.
    let limited = hub.metrics.orders_rate_limited.get();
    let mut statuses = Vec::new();
    for _ in 0..3 {
        let (s, v) = api.post("/orders", &trade, order("A1")).await;
        statuses.push((s, code(&v).to_string()));
    }
    assert_eq!(
        statuses,
        vec![
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable".to_string()),
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable".to_string()),
            (StatusCode::TOO_MANY_REQUESTS, "rate_limited".to_string()),
        ]
    );
    assert_eq!(hub.metrics.orders_rate_limited.get(), limited + 1);
}

#[tokio::test]
async fn request_rate_limit_per_key() {
    let cfg = ClientGatewayConfig {
        rest_requests_per_second: 1,
        rest_burst: 3,
        ..Default::default()
    };
    let hub = Hub::new(cfg, Authenticator::hs256(KEY), Vec::new(), None);
    let api = Api::new(serve(hub.clone()).await);
    let a = key_token(&["A1"], "read");
    let b = key_token(&["A1"], "trade");
    for _ in 0..3 {
        assert_eq!(api.get("/symbols", &a).await.0, StatusCode::OK);
    }
    let (s, v) = api.get("/symbols", &a).await;
    assert_eq!(
        (s, code(&v)),
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited")
    );
    // another key (sid) has its own bucket
    assert_eq!(api.get("/symbols", &b).await.0, StatusCode::OK);
    assert_eq!(hub.metrics.rest_rate_limited.get(), 1);
}

#[tokio::test]
async fn orders_positions_and_idempotency_against_the_core() {
    let cfg = ClientGatewayConfig {
        rest_requests_per_second: 1_000,
        rest_burst: 1_000,
        ..Default::default()
    };
    let demo = Demo::start_with(
        cfg,
        Authenticator::hs256(KEY),
        DemoOptions {
            tick_ms: Some(20),
            volatility_ticks: Some(1),
            data_dir: None,
        },
    )
    .await
    .unwrap();
    let api = Api::new(serve(demo.hub.clone()).await);
    let t = key_token(&["DEMO-H1"], "trade");

    // the account's (marked-up) quote, once the feed is up
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let q = loop {
        let (s, v) = api.get("/quotes/EURUSD", &t).await;
        if s == StatusCode::OK {
            break v;
        }
        assert_eq!(code(&v), "no_quote", "{v}");
        assert!(tokio::time::Instant::now() < deadline, "no quote");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let (bid, ask) = (fx(&q["bid"]), fx(&q["ask"]));
    assert!(bid < ask, "{q}");
    let (s, v) = api.get("/quotes/XXXYYY", &t).await;
    assert_eq!((s, code(&v)), (StatusCode::NOT_FOUND, "unknown_symbol"));

    let (s, v) = api.get("/accounts", &t).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let acc = &v["accounts"][0];
    assert_eq!(acc["id"], "DEMO-H1");
    assert_eq!(acc["margin_mode"], "hedging");
    assert_eq!(acc["currency"], "USD");
    assert_eq!(fx(&acc["balance"]), Fixed::from_int(10_000));
    let (s, v) = api.get("/accounts/DEMO-H1", &t).await;
    assert_eq!((s, &v["account"]["leverage"]), (StatusCode::OK, &json!(30)));

    // --- idempotent client order id: a retry returns the same order ---
    let limit = json!({
        "symbol": "EURUSD", "side": "buy", "type": "limit", "qty": "100000",
        "limit_price": off(bid, "-0.0200"), "client_order_id": "rest-lim-1",
    });
    let (s, first) = api.post("/orders", &t, limit.clone()).await;
    assert_eq!(s, StatusCode::CREATED, "{first}");
    assert_eq!(first["replayed"], false);
    assert_eq!(first["account"], "DEMO-H1");
    let order_id = first["order_id"].as_str().unwrap().to_string();
    let (s, again) = api.post("/orders", &t, limit.clone()).await;
    assert_eq!(s, StatusCode::OK, "{again}");
    assert_eq!(again["replayed"], true);
    assert_eq!(again["order_id"], order_id.as_str());
    assert_eq!(again["order"]["status"], "new");
    assert_eq!(again["order"]["type"], "limit");
    // the same id for a different order is a conflict, not a second order
    let mut other = limit.clone();
    other["side"] = json!("sell");
    let (s, v) = api.post("/orders", &t, other).await;
    assert_eq!(
        (s, code(&v)),
        (StatusCode::CONFLICT, "client_order_id_conflict"),
        "{v}"
    );
    // the Idempotency-Key header works the same way
    let mut keyed = limit.clone();
    keyed.as_object_mut().unwrap().remove("client_order_id");
    let send_keyed = || {
        api.http
            .post(format!("{}/orders", api.base))
            .bearer_auth(&t)
            .header("Idempotency-Key", "rest-lim-2")
            .json(&keyed)
            .send()
    };
    assert_eq!(send_keyed().await.unwrap().status(), StatusCode::CREATED);
    assert_eq!(send_keyed().await.unwrap().status(), StatusCode::OK);
    let (s, v) = api.get("/orders", &t).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = v["orders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["client_order_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["rest-lim-1", "rest-lim-2"], "one order per id");

    // modify (limit price), then cancel by engine order id and by client id
    let (s, v) = api
        .call(
            Method::PATCH,
            "/orders/rest-lim-1",
            Some(&t),
            Some(json!({"limit_price": off(bid, "-0.0300")})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, v) = api.get("/orders", &t).await;
    assert_eq!(
        fx(&v["orders"][0]["limit_price"]),
        off(bid, "-0.0300").parse::<Fixed>().unwrap()
    );
    let (s, v) = api
        .call(
            Method::DELETE,
            &format!("/orders/{order_id}"),
            Some(&t),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["client_order_id"], "rest-lim-1");
    let (s, _) = api
        .call(Method::DELETE, "/orders/rest-lim-2", Some(&t), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = api
        .call(Method::DELETE, "/orders/rest-lim-2", Some(&t), None)
        .await;
    assert!(s.is_client_error(), "cancel twice: {s} {v}");
    let (_, v) = api.get("/orders", &t).await;
    assert_eq!(v["orders"], json!([]));
    let (_, v) = api.get("/orders?history=1", &t).await;
    assert!(v["orders"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["client_order_id"] == "rest-lim-1" && o["status"] == "canceled"));

    // --- market order with SL/TP -> position; PATCH keeps omitted levels ---
    let (s, v) = api
        .post(
            "/orders",
            &t,
            json!({
                "symbol": "EURUSD", "side": "buy", "qty": 100000,
                "sl": off(bid, "-0.0100"), "tp": off(ask, "0.0100"),
                "client_order_id": "rest-mkt-1",
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let pos = loop {
        let (s, v) = api.get("/positions?account=DEMO-H1", &t).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        if let Some(p) = v["positions"].as_array().and_then(|a| a.first()) {
            break p.clone();
        }
        assert!(tokio::time::Instant::now() < deadline, "no position");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(pos["side"], "buy");
    assert_eq!(fx(&pos["qty"]), Fixed::from_int(100_000));
    assert_eq!(
        fx(&pos["sl"]),
        off(bid, "-0.0100").parse::<Fixed>().unwrap()
    );
    let pid = pos["id"].as_str().unwrap().to_string();
    let (s, v) = api
        .call(
            Method::PATCH,
            &format!("/positions/{pid}"),
            Some(&t),
            Some(json!({"tp": null})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["tp"], Value::Null);
    assert_eq!(fx(&v["sl"]), fx(&pos["sl"]), "omitted SL kept");
    let (_, v) = api.get("/positions", &t).await;
    assert_eq!(v["positions"][0]["tp"], Value::Null);
    assert_eq!(v["positions"][0]["sl"], pos["sl"]);

    // --- partial close, idempotent like orders ---
    let close = json!({"qty": "50000", "client_order_id": "rest-close-1"});
    let path = format!("/positions/{pid}/close");
    let (s, v) = api.post(&path, &t, close.clone()).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, v) = api.post(&path, &t, close.clone()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["replayed"], true);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let (_, v) = api.get("/positions", &t).await;
        if v["positions"][0]["qty"] == "50000" {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "not reduced: {v}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // --- deal history, paged, with a time filter ---
    let mut deals = Vec::new();
    let mut cursor = String::new();
    for _ in 0..5 {
        let (s, v) = api
            .get(&format!("/deals?limit=1&cursor={cursor}"), &t)
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        deals.extend(v["deals"].as_array().unwrap().iter().cloned());
        match v["next_cursor"].as_str() {
            Some(c) => cursor = c.to_string(),
            None => break,
        }
    }
    let entries: Vec<&str> = deals.iter().map(|d| d["entry"].as_str().unwrap()).collect();
    assert_eq!(entries, vec!["in", "out"], "{deals:?}");
    assert_eq!(deals[1]["client_order_id"], "rest-close-1");
    assert_eq!(fx(&deals[1]["qty"]), Fixed::from_int(50_000));
    let tomorrow = domain::now_ns() / 1_000_000 + 86_400_000;
    let (_, v) = api.get(&format!("/deals?from={tomorrow}"), &t).await;
    assert_eq!(v["deals"], json!([]));
    let (_, v) = api.get("/deals?from=2020-01-01T00:00:00Z", &t).await;
    assert_eq!(v["deals"].as_array().unwrap().len(), 2);
    let (s, _) = api.get("/deals?from=yesterday", &t).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // a read key sees all of it but cannot trade, even with a core
    let read = key_token(&["DEMO-H1"], "read");
    let (s, v) = api.get("/positions", &read).await;
    assert_eq!(
        (s, v["positions"].as_array().unwrap().len()),
        (StatusCode::OK, 1)
    );
    let (s, _) = api.post(&path, &read, json!({})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (_, v) = api.get("/positions", &t).await;
    assert_eq!(v["positions"][0]["qty"], "50000", "read key closed nothing");

    demo.shutdown().await;
}
