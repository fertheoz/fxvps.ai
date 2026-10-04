//! client-gateway against a real (in-process) identity service: RS256 tokens from
//! the JWKS URL, iss/aud validation, key rotation picked up on an unknown `kid`, and
//! a WebSocket login with an identity-issued token.

use std::sync::Arc;
use std::time::Duration;

use client_gateway::auth::{AuthError, Authenticator};
use client_gateway::{ClientGatewayConfig, Hub};
use client_proto::*;
use futures_util::{SinkExt, StreamExt};
use identity::keys::KeyRing;
use identity::mail::MemoryMailer;
use identity::store::MemoryStore;
use identity::{App, Config};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

const SERVICE: &str = "svc-token-for-tests-0123456789-0123456789";

struct Idp {
    base: String,
    mail: Arc<MemoryMailer>,
    http: reqwest::Client,
    cfg: Config,
}

impl Idp {
    async fn start() -> Idp {
        let cfg = Config {
            argon2_m_kib: 256,
            argon2_t: 1,
            ip_requests_per_minute: 10_000,
            service_token: Some(SERVICE.into()),
            issuer: "https://id.test".into(),
            ..Config::default()
        };
        let mail = Arc::new(MemoryMailer::default());
        let app = App::new(
            cfg.clone(),
            Arc::new(MemoryStore::new()),
            KeyRing::ephemeral().unwrap(),
            mail.clone(),
        )
        .unwrap();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, identity::http::router(app)).await });
        Idp {
            base,
            mail,
            http: reqwest::Client::new(),
            cfg,
        }
    }

    async fn post(&self, path: &str, bearer: Option<&str>, body: Value) -> Value {
        let mut r = self.http.post(format!("{}{path}", self.base)).json(&body);
        if let Some(b) = bearer {
            r = r.bearer_auth(b);
        }
        let r = r.send().await.unwrap();
        assert!(r.status().is_success(), "{path}: {}", r.status());
        r.json().await.unwrap_or(Value::Null)
    }

    /// Registered, verified user owning `accounts`; returns a fresh access token.
    async fn user(&self, email: &str, accounts: &[&str]) -> String {
        self.post(
            "/v1/register",
            None,
            json!({"email": email, "password": "long enough password"}),
        )
        .await;
        let link = self.mail.last_to(email).unwrap().link.unwrap();
        let token = link.split_once("verify_email=").unwrap().1;
        self.post("/v1/verify-email", None, json!({ "token": token }))
            .await;
        for a in accounts {
            self.post(
                "/v1/admin/accounts/link",
                Some(SERVICE),
                json!({"email": email, "account_id": a}),
            )
            .await;
        }
        self.login(email).await
    }

    async fn login(&self, email: &str) -> String {
        let r = self
            .post(
                "/v1/login",
                None,
                json!({"email": email, "password": "long enough password"}),
            )
            .await;
        r["access_token"].as_str().unwrap().to_string()
    }

    fn gateway_auth(&self, audience: &str) -> Authenticator {
        Authenticator::jwks_url(
            &format!("{}/.well-known/jwks.json", self.base),
            Duration::ZERO,
        )
        .unwrap()
        .with_validation(Some(self.cfg.issuer.clone()), Some(audience.into()))
    }
}

#[tokio::test]
async fn gateway_verifies_identity_tokens_across_rotation() {
    let idp = Idp::start().await;
    let token = idp.user("alice@example.com", &["ACC-1", "ACC-2"]).await;

    let auth = idp.gateway_auth("fxvps");
    assert_eq!(auth.refresh().await.unwrap(), 1);
    let c = auth.verify(&token).unwrap();
    assert_eq!(c.accounts, vec!["ACC-1", "ACC-2"]);
    assert_eq!(c.roles, vec!["client"]);
    assert_eq!(c.amr, vec!["pwd"]);
    assert!(c.may_access("ACC-1") && !c.may_access("ACC-3"));

    // aud / iss are enforced
    let wrong_aud = idp.gateway_auth("someone-else");
    wrong_aud.refresh().await.unwrap();
    assert!(matches!(
        wrong_aud.verify(&token),
        Err(AuthError::Invalid(_))
    ));
    let wrong_iss = Authenticator::jwks_url(
        &format!("{}/.well-known/jwks.json", idp.base),
        Duration::ZERO,
    )
    .unwrap()
    .with_validation(Some("https://evil.test".into()), Some("fxvps".into()));
    wrong_iss.refresh().await.unwrap();
    assert!(wrong_iss.verify(&token).is_err());

    // rotate the signing key: the unknown kid triggers a JWKS refetch
    idp.post("/v1/admin/keys/rotate", Some(SERVICE), json!({}))
        .await;
    let rotated = idp.login("alice@example.com").await;
    let first = auth.verify(&rotated);
    assert!(matches!(first, Err(AuthError::NoKey { .. })), "{first:?}");
    let mut ok = None;
    for _ in 0..100 {
        if let Ok(c) = auth.verify(&rotated) {
            ok = Some(c);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(ok.expect("rotated key picked up").sub, c.sub);
    // tokens signed with the retired key stay valid until they expire
    assert!(auth.verify(&token).is_ok());

    // a JWKS outage keeps the cached keys
    let dead = Authenticator::jwks_url("http://127.0.0.1:1/jwks.json", Duration::ZERO).unwrap();
    assert!(matches!(dead.refresh().await, Err(AuthError::Fetch(_))));
    assert!(auth.verify(&rotated).is_ok());
}

#[tokio::test]
async fn websocket_login_with_identity_token() {
    let idp = Idp::start().await;
    let token = idp.user("bob@example.com", &["ACC-7"]).await;
    let auth = idp.gateway_auth("fxvps");
    auth.refresh().await.unwrap();
    let hub = Hub::new(
        ClientGatewayConfig::default(),
        auth,
        Vec::<String>::new(),
        None,
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(client_gateway::serve(hub, l));

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    let mut seq = 0;
    let mut send = |body: Body| {
        seq += 1;
        Message::Binary(Envelope::new(seq, body).to_protobuf().into())
    };
    ws.send(send(Body::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        client_name: "test".into(),
        max_quote_hz: 1,
    })))
    .await
    .unwrap();
    ws.send(send(Body::Auth(Auth { token }))).await.unwrap();
    loop {
        let m = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("frame")
            .unwrap()
            .unwrap();
        let Message::Binary(b) = m else { continue };
        match Envelope::from_protobuf(&b).unwrap().body.unwrap() {
            Body::AuthOk(ok) => {
                assert_eq!(ok.account_ids, vec!["ACC-7".to_string()]);
                break;
            }
            Body::Error(e) => panic!("auth failed: {e:?}"),
            _ => continue,
        }
    }
}
