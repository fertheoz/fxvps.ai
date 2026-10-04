#![allow(dead_code)]

use std::sync::{Arc, OnceLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use identity::keys::KeyRing;
use identity::mail::MemoryMailer;
use identity::store::{MemoryStore, Store};
use identity::{App, Config};
use rsa::RsaPrivateKey;
use serde_json::{json, Value};
use tower::ServiceExt;

pub const PW: &str = "correct horse battery";

/// One RSA key per test binary (generation is slow in debug builds).
pub fn test_key() -> RsaPrivateKey {
    static K: OnceLock<RsaPrivateKey> = OnceLock::new();
    K.get_or_init(|| identity::keys::generate_rsa().unwrap())
        .clone()
}

pub fn test_config() -> Config {
    Config {
        argon2_m_kib: 256,
        argon2_t: 1,
        ip_requests_per_minute: 10_000,
        admin_require_mfa: false,
        service_token: Some("svc-token-0123456789-0123456789-0123456789".into()),
        ..Config::default()
    }
}

pub struct H {
    pub app: Arc<App>,
    pub router: Router,
    pub mail: Arc<MemoryMailer>,
}

pub struct Resp {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub body: Value,
}

impl H {
    pub fn new() -> Self {
        Self::with(Arc::new(MemoryStore::new()), test_config())
    }

    pub fn with(store: Arc<dyn Store>, cfg: Config) -> Self {
        let mail = Arc::new(MemoryMailer::default());
        let keys = KeyRing::new(&test_key(), &[], true).unwrap();
        let app = App::new(cfg, store, keys, mail.clone()).unwrap();
        H {
            router: identity::http::router(app.clone()),
            app,
            mail,
        }
    }

    pub async fn req(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        headers: &[(&str, &str)],
    ) -> Resp {
        let mut b = Request::builder().method(method).uri(path);
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let req = match body {
            Some(v) => b
                .header("content-type", "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => b.body(Body::empty()).unwrap(),
        };
        let r = self.router.clone().oneshot(req).await.unwrap();
        let status = r.status();
        let headers = r.headers().clone();
        let bytes = r.into_body().collect().await.unwrap().to_bytes();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Resp {
            status,
            headers,
            body,
        }
    }

    pub async fn post(&self, path: &str, body: Value) -> Resp {
        self.req("POST", path, Some(body), &[]).await
    }

    pub async fn get_auth(&self, path: &str, token: &str) -> Resp {
        self.req(
            "GET",
            path,
            None,
            &[("authorization", &format!("Bearer {token}"))],
        )
        .await
    }

    pub async fn post_auth(&self, path: &str, token: &str, body: Value) -> Resp {
        self.req(
            "POST",
            path,
            Some(body),
            &[("authorization", &format!("Bearer {token}"))],
        )
        .await
    }

    /// Token from the last mailed link with `param`.
    pub fn link_token(&self, email: &str, param: &str) -> String {
        let link = self
            .mail
            .last_to(email)
            .and_then(|m| m.link)
            .expect("mail with link");
        let (_, t) = link
            .split_once(&format!("{param}="))
            .expect("param in link");
        t.to_string()
    }

    /// Registers and verifies; returns nothing.
    pub async fn register_verified(&self, email: &str) {
        let r = self
            .post("/v1/register", json!({"email": email, "password": PW}))
            .await;
        assert_eq!(r.status, StatusCode::ACCEPTED);
        let t = self.link_token(email, "verify_email");
        let r = self.post("/v1/verify-email", json!({"token": t})).await;
        assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    }

    /// Password login (no 2FA) → token response body.
    pub async fn login(&self, email: &str) -> Value {
        let r = self
            .post("/v1/login", json!({"email": email, "password": PW}))
            .await;
        assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
        r.body
    }

    pub fn service_token(&self) -> String {
        self.app.cfg.service_token.clone().unwrap()
    }
}

pub fn totp_code(secret: &str, email: &str, step_offset: i64) -> String {
    let t = identity::crypto::totp(secret, email).unwrap();
    let now = identity::now() as u64;
    let step = (now / 30) as i64 + step_offset;
    t.generate(step as u64 * 30)
}
