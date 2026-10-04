//! Admin API with identity-issued RS256 tokens (JWKS file / URL), iss/aud and
//! the MFA (`amr`) requirement for mutating permissions.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use core_engine::admin::auth::{unix_now, Authenticator};
use core_engine::admin::{self, AdminConfig};
use core_engine::{spawn, Settings};
use http_body_util::BodyExt;
use identity::keys::KeyRing;
use jsonwebtoken::jwk::JwkSet;
use serde_json::{json, Value};
use tower::ServiceExt;

fn identity_token(ring: &KeyRing, iss: &str, aud: &str, roles: &[&str], amr: &[&str]) -> String {
    ring.sign(&json!({
        "iss": iss, "aud": aud, "sub": "user-1", "exp": unix_now() + 60, "iat": unix_now(),
        "jti": "j", "sid": "s", "email": "a@b.c", "accounts": [], "roles": roles, "amr": amr,
    }))
    .unwrap()
}

async fn call(
    app: &Router,
    m: Method,
    path: &str,
    tok: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder()
        .method(m)
        .uri(path)
        .header("authorization", format!("Bearer {tok}"));
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let st = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread")]
async fn identity_jwks_iss_aud_and_mfa() {
    let ring = KeyRing::ephemeral().unwrap();
    let set: JwkSet = serde_json::from_value(ring.jwks()).unwrap();
    let auth = Authenticator::jwks(&set)
        .unwrap()
        .with_validation(Some("https://id.test".into()), Some("fxvps".into()))
        .with_require_mfa(true);
    let dir = tempfile::tempdir().unwrap();
    let (h, join) = spawn(Settings::new(dir.path())).unwrap();
    let app = admin::app(h.clone(), auth, AdminConfig::new(dir.path())).unwrap();

    let pwd_only = identity_token(
        &ring,
        "https://id.test",
        "fxvps",
        &["client", "admin"],
        &["pwd"],
    );
    let mfa = identity_token(
        &ring,
        "https://id.test",
        "fxvps",
        &["admin"],
        &["pwd", "otp"],
    );
    let (s, me) = call(&app, Method::GET, "/v1/me", &pwd_only, None).await;
    assert_eq!(s, StatusCode::OK, "{me}");
    assert_eq!(me["role"], "admin");
    assert_eq!(me["sub"], "user-1");
    // wrong issuer / audience
    for t in [
        identity_token(&ring, "https://evil", "fxvps", &["admin"], &["otp"]),
        identity_token(&ring, "https://id.test", "other", &["admin"], &["otp"]),
    ] {
        assert_eq!(
            call(&app, Method::GET, "/v1/me", &t, None).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    // a key that is not in the JWKS
    let other = KeyRing::ephemeral().unwrap();
    let t = identity_token(&other, "https://id.test", "fxvps", &["admin"], &["otp"]);
    assert_eq!(
        call(&app, Method::GET, "/v1/me", &t, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    // mutations need MFA; views do not
    let settings = json!({"brokerName": "x broker", "baseCurrency": "USD", "fourEyesThreshold": 1000000,
        "sessionTimeoutMin": 30, "requireMfa": true, "defaultBook": "A"});
    let (s, v) = call(
        &app,
        Method::PUT,
        "/v1/settings",
        &pwd_only,
        Some(settings.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["code"], "mfa_required");
    assert_eq!(
        call(&app, Method::GET, "/v1/settings", &pwd_only, None)
            .await
            .0,
        StatusCode::OK
    );
    let (s, v) = call(&app, Method::PUT, "/v1/settings", &mfa, Some(settings)).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // JWKS URL: keys fetched from the identity endpoint
    let jwks = ring.jwks();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/.well-known/jwks.json", l.local_addr().unwrap());
    let srv = Router::new().route(
        "/.well-known/jwks.json",
        axum::routing::get(move || {
            let j = jwks.clone();
            async move { axum::Json(j) }
        }),
    );
    tokio::spawn(async move { axum::serve(l, srv).await });
    let remote = Authenticator::jwks(&JwkSet { keys: vec![] })
        .unwrap()
        .with_validation(Some("https://id.test".into()), Some("fxvps".into()));
    assert!(remote.verify(&mfa).is_err());
    assert_eq!(remote.refresh_from(&url).await.unwrap(), 1);
    assert_eq!(remote.verify(&mfa).unwrap().sub, "user-1");

    h.shutdown();
    join.join().unwrap();
}
