//! End-to-end auth flows against the HTTP router (in-memory store), plus the same
//! flow against PostgreSQL when `DATABASE_URL` is set.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::*;
use identity::store::{MemoryStore, Store};
use serde_json::{json, Value};

async fn full_flow(h: &H) {
    let email = "Alice@Example.com";
    let norm = "alice@example.com";

    // register → login refused until the email is verified
    let r = h
        .post("/v1/register", json!({"email": email, "password": PW}))
        .await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    let r = h
        .post("/v1/login", json!({"email": email, "password": PW}))
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.body["error"], "email_not_verified");
    // registering again does not reveal the account exists
    let r = h
        .post("/v1/register", json!({"email": norm, "password": PW}))
        .await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    // invalid token rejected; real one works once
    let r = h.post("/v1/verify-email", json!({"token": "nope"})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = h
        .post("/v1/verify-email/resend", json!({"email": norm}))
        .await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    let vt = h.link_token(norm, "verify_email");
    assert_eq!(
        h.post("/v1/verify-email", json!({"token": vt}))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        h.post("/v1/verify-email", json!({"token": vt}))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );

    // password login → access token with claims
    let tok = h.login(email).await;
    let access = tok["access_token"].as_str().unwrap().to_string();
    let c = h.app.verify_access(&access).expect("valid access token");
    assert_eq!(c.email, norm);
    assert_eq!(c.roles, vec!["client"]);
    assert_eq!(c.amr, vec!["pwd"]);
    assert!(c.accounts.is_empty());
    assert_eq!(c.iss, h.app.cfg.issuer);
    assert_eq!(c.aud, "fxvps");
    let me = h.get_auth("/v1/me", &access).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.body["email"], norm);
    assert_eq!(me.body["totp_enabled"], false);
    assert_eq!(
        h.req("GET", "/v1/me", None, &[]).await.status,
        StatusCode::UNAUTHORIZED
    );

    // link a trading account (service token), visible after refresh
    let r = h
        .post_auth(
            "/v1/admin/accounts/link",
            &h.service_token(),
            json!({"email": norm, "account_id": "ACC-1"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let rt1 = tok["refresh_token"].as_str().unwrap().to_string();
    let r = h
        .post("/v1/token/refresh", json!({"refresh_token": rt1}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["accounts"], json!(["ACC-1"]));
    let rt2 = r.body["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(rt1, rt2);
    let access2 = r.body["access_token"].as_str().unwrap().to_string();
    assert_eq!(
        h.app.verify_access(&access2).unwrap().accounts,
        vec!["ACC-1"]
    );
    let r = h.get_auth("/v1/accounts", &access2).await;
    assert_eq!(r.body["accounts"], json!(["ACC-1"]));

    // refresh token reuse → family revoked (the fresh token dies too)
    let r = h
        .post("/v1/token/refresh", json!({"refresh_token": rt1}))
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = h
        .post("/v1/token/refresh", json!({"refresh_token": rt2}))
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // TOTP enrolment
    let access = h.login(email).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let r = h.post_auth("/v1/2fa/totp/enroll", &access, json!({})).await;
    assert_eq!(r.status, StatusCode::OK);
    let secret = r.body["secret"].as_str().unwrap().to_string();
    assert!(r.body["otpauth_url"]
        .as_str()
        .unwrap()
        .starts_with("otpauth://totp/"));
    let r = h
        .post_auth("/v1/2fa/totp/confirm", &access, json!({"code": "000000x"}))
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = h
        .post_auth(
            "/v1/2fa/totp/confirm",
            &access,
            json!({"code": totp_code(&secret, norm, 0)}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let codes: Vec<String> = serde_json::from_value(r.body["recovery_codes"].clone()).unwrap();
    assert_eq!(codes.len(), 10);

    // login now requires the second factor
    let r = h
        .post("/v1/login", json!({"email": email, "password": PW}))
        .await;
    assert_eq!(r.body["mfa_required"], true);
    assert!(r.body.get("access_token").is_none());
    let mfa = r.body["mfa_token"].as_str().unwrap().to_string();
    let r = h
        .post("/v1/login/2fa", json!({"mfa_token": mfa, "code": "123456"}))
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    // the code used to confirm enrolment cannot be replayed
    let r = h
        .post(
            "/v1/login/2fa",
            json!({"mfa_token": mfa, "code": totp_code(&secret, norm, 0)}),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = h
        .post(
            "/v1/login/2fa",
            json!({"mfa_token": mfa, "code": totp_code(&secret, norm, 1)}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let c = h
        .app
        .verify_access(r.body["access_token"].as_str().unwrap())
        .unwrap();
    assert!(c.amr.contains(&"otp".to_string()) && c.amr.contains(&"mfa".to_string()));
    // MFA token is single use
    let r = h
        .post(
            "/v1/login/2fa",
            json!({"mfa_token": mfa, "code": totp_code(&secret, norm, 1)}),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // recovery code login (single use)
    let mfa = h
        .post("/v1/login", json!({"email": email, "password": PW}))
        .await
        .body["mfa_token"]
        .as_str()
        .unwrap()
        .to_string();
    let r = h
        .post(
            "/v1/login/2fa",
            json!({"mfa_token": mfa, "recovery_code": codes[0].to_uppercase()}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let access = r.body["access_token"].as_str().unwrap().to_string();
    let refresh = r.body["refresh_token"].as_str().unwrap().to_string();
    let mfa = h
        .post("/v1/login", json!({"email": email, "password": PW}))
        .await
        .body["mfa_token"]
        .as_str()
        .unwrap()
        .to_string();
    let r = h
        .post(
            "/v1/login/2fa",
            json!({"mfa_token": mfa, "recovery_code": codes[0]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        h.get_auth("/v1/me", &access).await.body["recovery_codes_left"],
        9
    );

    // logout revokes the session; revoke-all kills the rest
    let r = h
        .post("/v1/token/revoke", json!({"refresh_token": refresh}))
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = h
        .post("/v1/token/refresh", json!({"refresh_token": refresh}))
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = h
        .post_auth("/v1/sessions/revoke-all", &access, json!({}))
        .await;
    assert_eq!(r.status, StatusCode::OK);

    // audit trail
    let r = h
        .get_auth(
            &format!("/v1/admin/audit?user_id={}", c.sub),
            &h.service_token(),
        )
        .await;
    let events: Vec<String> = r.body["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event"].as_str().unwrap().to_string())
        .collect();
    for e in [
        "register",
        "email_verified",
        "login_success",
        "refresh_token_reuse",
        "totp_enabled",
        "login_failed",
        "recovery_code_used",
        "logout",
        "account_linked",
    ] {
        assert!(
            events.iter().any(|x| x == e),
            "missing audit event {e}: {events:?}"
        );
    }
}

#[tokio::test]
async fn full_flow_memory() {
    full_flow(&H::new()).await;
}

#[tokio::test]
async fn full_flow_postgres() {
    let Some(h) = pg_harness().await else {
        eprintln!("DATABASE_URL not set; skipping PostgreSQL flow");
        return;
    };
    full_flow(&h).await;
}

/// Fresh schema per run so tests are repeatable.
async fn pg_harness() -> Option<H> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let schema = format!("t{}", identity::crypto::random_id().replace('-', ""));
    let admin = identity::pg::PgStore::connect_with_schema(&url, &schema)
        .await
        .unwrap();
    Some(H::with(Arc::new(admin), test_config()))
}

#[tokio::test]
async fn lockout_after_failed_logins() {
    let h = H::new();
    h.register_verified("bob@example.com").await;
    for _ in 0..5 {
        let r = h
            .post(
                "/v1/login",
                json!({"email": "bob@example.com", "password": "wrong password!"}),
            )
            .await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    // locked even with the right password
    let r = h
        .post(
            "/v1/login",
            json!({"email": "bob@example.com", "password": PW}),
        )
        .await;
    assert_eq!(r.status, StatusCode::LOCKED);
    assert_eq!(r.body["error"], "account_locked");
    // unknown email: same response as a wrong password
    let r = h
        .post(
            "/v1/login",
            json!({"email": "nobody@example.com", "password": PW}),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.body["error"], "invalid_credentials");
    // password reset unlocks and revokes sessions
    assert_eq!(
        h.post("/v1/password/forgot", json!({"email": "bob@example.com"}))
            .await
            .status,
        StatusCode::ACCEPTED
    );
    let t = h.link_token("bob@example.com", "reset_password");
    let r = h
        .post(
            "/v1/password/reset",
            json!({"token": t, "password": "short"}),
        )
        .await;
    assert_eq!(r.body["error"], "weak_password");
    let r = h
        .post(
            "/v1/password/reset",
            json!({"token": t, "password": "a brand new passphrase"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let r = h
        .post(
            "/v1/login",
            json!({"email": "bob@example.com", "password": "a brand new passphrase"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    // old password no longer works; reset link is single use
    let r = h
        .post(
            "/v1/login",
            json!({"email": "bob@example.com", "password": PW}),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = h
        .post(
            "/v1/password/reset",
            json!({"token": t, "password": "another passphrase"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn password_reset_revokes_sessions() {
    let h = H::new();
    h.register_verified("carol@example.com").await;
    let rt = h.login("carol@example.com").await["refresh_token"]
        .as_str()
        .unwrap()
        .to_string();
    h.post("/v1/password/forgot", json!({"email": "carol@example.com"}))
        .await;
    let t = h.link_token("carol@example.com", "reset_password");
    h.post(
        "/v1/password/reset",
        json!({"token": t, "password": "new long passphrase"}),
    )
    .await;
    let r = h
        .post("/v1/token/refresh", json!({"refresh_token": rt}))
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn totp_lockout_on_guessing() {
    let h = H::new();
    h.register_verified("dan@example.com").await;
    let access = h.login("dan@example.com").await["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let secret = h
        .post_auth("/v1/2fa/totp/enroll", &access, json!({}))
        .await
        .body["secret"]
        .as_str()
        .unwrap()
        .to_string();
    h.post_auth(
        "/v1/2fa/totp/confirm",
        &access,
        json!({"code": totp_code(&secret, "dan@example.com", 0)}),
    )
    .await;
    let mut locked = false;
    for _ in 0..6 {
        let r = h
            .post(
                "/v1/login",
                json!({"email": "dan@example.com", "password": PW}),
            )
            .await;
        if r.status == StatusCode::LOCKED {
            locked = true;
            break;
        }
        let mfa = r.body["mfa_token"].as_str().unwrap().to_string();
        let r = h
            .post("/v1/login/2fa", json!({"mfa_token": mfa, "code": "000000"}))
            .await;
        assert!(matches!(
            r.status,
            StatusCode::UNAUTHORIZED | StatusCode::LOCKED
        ));
    }
    assert!(locked, "second-factor guessing must lock the account");
}

#[tokio::test]
async fn cookie_mode_is_csrf_safe() {
    let h = H::new();
    h.register_verified("erin@example.com").await;
    let r = h
        .post(
            "/v1/login",
            json!({"email": "erin@example.com", "password": PW, "session": "cookie"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(
        r.body.get("refresh_token").is_none(),
        "web mode must not expose the refresh token"
    );
    let sc = r
        .headers
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(sc.contains("HttpOnly") && sc.contains("SameSite=Strict") && sc.contains("Secure"));
    assert!(sc.contains("Path=/v1/token"));
    let cookie = sc.split(';').next().unwrap().to_string();

    // no CSRF header → refused
    let r = h
        .req("POST", "/v1/token/refresh", None, &[("cookie", &cookie)])
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // foreign origin → refused
    let r = h
        .req(
            "POST",
            "/v1/token/refresh",
            None,
            &[
                ("cookie", &cookie),
                ("x-fxvps-csrf", "1"),
                ("origin", "https://evil.example"),
            ],
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // allowed origin + header → rotated cookie
    let r = h
        .req(
            "POST",
            "/v1/token/refresh",
            None,
            &[
                ("cookie", &cookie),
                ("x-fxvps-csrf", "1"),
                ("origin", "http://localhost:5173"),
            ],
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert!(r.body.get("refresh_token").is_none());
    let sc2 = r
        .headers
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_ne!(sc2.split(';').next().unwrap(), cookie);
    // logout clears the cookie
    let cookie2 = sc2.split(';').next().unwrap().to_string();
    let r = h
        .req(
            "POST",
            "/v1/token/revoke",
            None,
            &[("cookie", &cookie2), ("x-fxvps-csrf", "1")],
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert!(r
        .headers
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
}

#[tokio::test]
async fn cors_preflight_allows_configured_origin_only() {
    let h = H::new();
    let pre = |o: &'static str| {
        let h = &h;
        async move {
            h.req(
                "OPTIONS",
                "/v1/login",
                None,
                &[
                    ("origin", o),
                    ("access-control-request-method", "POST"),
                    (
                        "access-control-request-headers",
                        "content-type,x-fxvps-csrf",
                    ),
                ],
            )
            .await
        }
    };
    let ok = pre("http://localhost:5173").await;
    assert_eq!(
        ok.headers.get("access-control-allow-origin").unwrap(),
        "http://localhost:5173"
    );
    assert_eq!(
        ok.headers.get("access-control-allow-credentials").unwrap(),
        "true"
    );
    let bad = pre("https://evil.example").await;
    assert!(bad.headers.get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn admin_endpoints_require_admin() {
    let mut cfg = test_config();
    cfg.bootstrap_admins = vec!["root@example.com".into()];
    let h = H::with(Arc::new(MemoryStore::new()), cfg);
    h.register_verified("root@example.com").await;
    h.register_verified("frank@example.com").await;
    h.register_verified("gina@example.com").await;
    let user = h.login("frank@example.com").await["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let admin = h.login("root@example.com").await["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(h
        .app
        .verify_access(&admin)
        .unwrap()
        .roles
        .contains(&"admin".to_string()));

    let link = json!({"email": "frank@example.com", "account_id": "ACC-9"});
    assert_eq!(
        h.post("/v1/admin/accounts/link", link.clone()).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.post_auth("/v1/admin/accounts/link", &user, link.clone())
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.post_auth(
            "/v1/admin/accounts/link",
            "wrong-service-token",
            link.clone()
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.post_auth("/v1/admin/accounts/link", &admin, link.clone())
            .await
            .status,
        StatusCode::OK
    );
    // idempotent
    assert_eq!(
        h.post_auth("/v1/admin/accounts/link", &admin, link.clone())
            .await
            .status,
        StatusCode::OK
    );
    // owned by someone else
    let r = h
        .post_auth(
            "/v1/admin/accounts/link",
            &admin,
            json!({"email": "gina@example.com", "account_id": "ACC-9"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    let r = h
        .post_auth(
            "/v1/admin/accounts/link",
            &admin,
            json!({"email": "ghost@example.com", "account_id": "X"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = h
        .get_auth("/v1/admin/users?email=frank@example.com", &admin)
        .await;
    assert_eq!(r.body["accounts"], json!(["ACC-9"]));

    // roles
    let r = h
        .post_auth(
            "/v1/admin/users/roles",
            &admin,
            json!({"email": "frank@example.com", "roles": ["superuser"]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = h
        .post_auth(
            "/v1/admin/users/roles",
            &h.service_token(),
            json!({"email": "frank@example.com", "roles": ["client", "dealer"]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let c = h
        .app
        .verify_access(
            h.login("frank@example.com").await["access_token"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(c.roles, vec!["client", "dealer"]);

    let r = h
        .post_auth(
            "/v1/admin/accounts/unlink",
            &admin,
            json!({"email": "frank@example.com", "account_id": "ACC-9"}),
        )
        .await;
    assert_eq!(r.body["removed"], true);
    assert_eq!(r.body["accounts"], json!([]));
    assert_eq!(
        h.get_auth("/v1/admin/audit", &user).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn jwks_discovery_and_rotation() {
    let h = H::new();
    let d = h
        .req("GET", "/.well-known/openid-configuration", None, &[])
        .await;
    assert_eq!(d.body["issuer"], h.app.cfg.issuer);
    assert_eq!(
        d.body["jwks_uri"],
        format!("{}/.well-known/jwks.json", h.app.cfg.issuer)
    );
    let j = h.req("GET", "/.well-known/jwks.json", None, &[]).await;
    let keys = j.body["keys"].as_array().unwrap().clone();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["alg"], "RS256");
    assert_eq!(keys[0]["kid"], h.app.keys.active_kid());
    assert!(
        keys[0].get("d").is_none(),
        "private material must never be published"
    );

    h.register_verified("hank@example.com").await;
    let old = h.login("hank@example.com").await["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let r = h
        .post_auth("/v1/admin/keys/rotate", &h.service_token(), json!({}))
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let new_kid = r.body["kid"].as_str().unwrap().to_string();
    let j = h.req("GET", "/.well-known/jwks.json", None, &[]).await;
    let kids: Vec<Value> = j.body["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["kid"].clone())
        .collect();
    assert_eq!(kids.len(), 2);
    assert_eq!(kids[0], new_kid);
    // old tokens stay valid; new ones are signed with the new key
    assert!(h.app.verify_access(&old).is_some());
    let new = h.login("hank@example.com").await["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let hdr = jsonwebtoken::decode_header(&new).unwrap();
    assert_eq!(hdr.kid.as_deref(), Some(new_kid.as_str()));
    assert_eq!(hdr.alg, jsonwebtoken::Algorithm::RS256);
}

#[tokio::test]
async fn ip_rate_limit() {
    let mut cfg = test_config();
    cfg.ip_requests_per_minute = 3;
    let h = H::with(Arc::new(MemoryStore::new()), cfg);
    let mut codes = Vec::new();
    for _ in 0..5 {
        codes.push(
            h.post(
                "/v1/login",
                json!({"email": "x@example.com", "password": PW}),
            )
            .await
            .status,
        );
    }
    assert_eq!(
        codes
            .iter()
            .filter(|c| **c == StatusCode::TOO_MANY_REQUESTS)
            .count(),
        2,
        "{codes:?}"
    );
    // non-auth endpoints are not throttled
    assert_eq!(
        h.req("GET", "/.well-known/jwks.json", None, &[])
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn input_validation() {
    let h = H::new();
    let r = h
        .post(
            "/v1/register",
            json!({"email": "not-an-email", "password": PW}),
        )
        .await;
    assert_eq!(r.body["error"], "invalid_email");
    let r = h
        .post(
            "/v1/register",
            json!({"email": "a@b.co", "password": "short"}),
        )
        .await;
    assert_eq!(r.body["error"], "weak_password");
    let r = h
        .post(
            "/v1/login/2fa",
            json!({"mfa_token": "bogus", "code": "123456"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn store_conformance_memory() {
    store_conformance(Arc::new(MemoryStore::new())).await;
}

#[tokio::test]
async fn store_conformance_postgres() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    let schema = format!("t{}", identity::crypto::random_id().replace('-', ""));
    let s = identity::pg::PgStore::connect_with_schema(&url, &schema)
        .await
        .unwrap();
    store_conformance(Arc::new(s)).await;
}

async fn store_conformance(s: Arc<dyn Store>) {
    use identity::store::*;
    let u = User {
        id: identity::crypto::random_id(),
        email: "s@example.com".into(),
        password_hash: "h".into(),
        email_verified: false,
        roles: vec!["client".into()],
        totp_secret: None,
        totp_enabled: false,
        totp_last_step: 0,
        recovery_codes: vec![],
        failed_logins: 0,
        locked_until: 0,
        created_at: 1,
    };
    s.create_user(&u).await.unwrap();
    assert!(matches!(s.create_user(&u).await, Err(StoreError::Conflict)));
    let mut u2 = s.user_by_email("s@example.com").await.unwrap().unwrap();
    assert_eq!(u2, u);
    u2.roles = vec!["admin".into(), "client".into()];
    u2.recovery_codes = vec!["a".into(), "b".into()];
    u2.totp_secret = Some("SECRET".into());
    s.update_user(&u2).await.unwrap();
    assert_eq!(s.user_by_id(&u.id).await.unwrap().unwrap(), u2);

    let t = OneTimeToken {
        hash: "th".into(),
        kind: TokenKind::Mfa,
        user_id: u.id.clone(),
        expires_at: 100,
        data: Some("{}".into()),
        attempts: 0,
    };
    s.put_token(&t).await.unwrap();
    assert!(s
        .get_token("th", TokenKind::VerifyEmail)
        .await
        .unwrap()
        .is_none());
    assert_eq!(s.bump_token_attempts("th").await.unwrap(), 1);
    assert_eq!(
        s.take_token("th", TokenKind::Mfa)
            .await
            .unwrap()
            .unwrap()
            .attempts,
        1
    );
    assert!(s.take_token("th", TokenKind::Mfa).await.unwrap().is_none());

    let rt = |h: &str| RefreshToken {
        hash: h.into(),
        user_id: u.id.clone(),
        family_id: "fam".into(),
        amr: vec!["pwd".into()],
        created_at: 1,
        expires_at: 1_000,
        revoked: false,
        rotated: false,
    };
    s.insert_refresh(&rt("r1")).await.unwrap();
    assert!(matches!(
        s.rotate_refresh("r1", &rt("r2"), 10).await.unwrap(),
        Rotation::Rotated(_)
    ));
    assert!(matches!(
        s.rotate_refresh("r1", &rt("r3"), 10).await.unwrap(),
        Rotation::Reused { .. }
    ));
    assert!(s.get_refresh("r2").await.unwrap().unwrap().revoked);
    assert!(s.get_refresh("r3").await.unwrap().is_none());
    assert!(matches!(
        s.rotate_refresh("nope", &rt("r4"), 10).await.unwrap(),
        Rotation::Invalid
    ));
    s.insert_refresh(&RefreshToken {
        family_id: "f2".into(),
        ..rt("r5")
    })
    .await
    .unwrap();
    assert!(matches!(
        s.rotate_refresh("r5", &rt("r6"), 5_000).await.unwrap(),
        Rotation::Invalid
    ));
    assert_eq!(s.revoke_user_sessions(&u.id).await.unwrap(), 1);

    let pk = PasskeyRecord {
        user_id: u.id.clone(),
        cred_id: "c1".into(),
        name: "key".into(),
        data: "{}".into(),
        created_at: 1,
    };
    s.add_passkey(&pk).await.unwrap();
    assert!(matches!(
        s.add_passkey(&pk).await,
        Err(StoreError::Conflict)
    ));
    s.update_passkey("c1", "{\"x\":1}").await.unwrap();
    assert_eq!(s.passkeys(&u.id).await.unwrap()[0].data, "{\"x\":1}");

    s.link_account(&u.id, "B").await.unwrap();
    s.link_account(&u.id, "A").await.unwrap();
    s.link_account(&u.id, "A").await.unwrap();
    assert_eq!(s.accounts(&u.id).await.unwrap(), vec!["A", "B"]);
    assert!(s.unlink_account(&u.id, "A").await.unwrap());
    assert!(!s.unlink_account(&u.id, "A").await.unwrap());

    for i in 0..3 {
        s.audit(&AuditEvent {
            ts: i,
            user_id: Some(u.id.clone()),
            event: format!("e{i}"),
            ip: None,
            detail: String::new(),
        })
        .await
        .unwrap();
    }
    let ev = s.audit_events(Some(&u.id), 2).await.unwrap();
    assert_eq!(
        ev.iter().map(|e| e.event.as_str()).collect::<Vec<_>>(),
        vec!["e2", "e1"]
    );
}

#[tokio::test]
async fn passkey_ceremony_boundaries() {
    let h = H::new();
    h.register_verified("ivy@example.com").await;
    let access = h.login("ivy@example.com").await["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    // login start without a registered passkey
    let r = h
        .post(
            "/v1/passkeys/login/start",
            json!({"email": "ivy@example.com"}),
        )
        .await;
    assert_eq!(r.body["error"], "no_passkeys");

    // registration options carry the RP and a user-verification requirement
    assert_eq!(
        h.post("/v1/passkeys/register/start", json!({}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let r = h
        .post_auth("/v1/passkeys/register/start", &access, json!({}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let pk = &r.body["options"]["publicKey"];
    assert_eq!(pk["rp"]["id"], "localhost");
    assert_eq!(pk["user"]["name"], "ivy@example.com");
    assert!(pk["challenge"].as_str().unwrap().len() >= 16);
    let reg = r.body["registration_id"].as_str().unwrap().to_string();

    // a forged attestation is rejected and the ceremony is single use
    let bogus = json!({
        "id": "AAAA", "rawId": "AAAA", "type": "public-key", "extensions": {},
        "response": {"attestationObject": "AAAA", "clientDataJSON": "AAAA"}
    });
    let r = h
        .post_auth(
            "/v1/passkeys/register/finish",
            &access,
            json!({"registration_id": reg, "credential": bogus}),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{:?}", r.body);
    assert_eq!(r.body["error"], "webauthn");
    let r = h
        .post_auth(
            "/v1/passkeys/register/finish",
            &access,
            json!({"registration_id": reg, "credential": bogus}),
        )
        .await;
    assert_eq!(r.body["error"], "invalid_registration");
}

#[tokio::test]
async fn totp_disable_guessing_locks_the_account() {
    let h = H::new();
    let email = "tess@example.com";
    h.register_verified(email).await;
    let access = h.login(email).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let secret = h
        .post_auth("/v1/2fa/totp/enroll", &access, json!({}))
        .await
        .body["secret"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        h.post_auth(
            "/v1/2fa/totp/confirm",
            &access,
            json!({"code": totp_code(&secret, email, 0)}),
        )
        .await
        .status,
        StatusCode::OK
    );
    // A stolen access token must not allow unlimited code guessing.
    let mut statuses = Vec::new();
    for _ in 0..6 {
        statuses.push(
            h.post_auth("/v1/2fa/totp/disable", &access, json!({"code": "000000"}))
                .await
                .status,
        );
    }
    assert!(statuses.contains(&StatusCode::LOCKED), "{statuses:?}");
    // locked: even the right code is refused, and password login is locked too
    assert_eq!(
        h.post_auth(
            "/v1/2fa/totp/disable",
            &access,
            json!({"code": totp_code(&secret, email, 0)}),
        )
        .await
        .status,
        StatusCode::LOCKED
    );
    assert_eq!(
        h.post("/v1/login", json!({"email": email, "password": PW}))
            .await
            .status,
        StatusCode::LOCKED
    );
}

#[tokio::test]
async fn admin_routes_need_mfa_and_sensitive_routes_are_rate_limited() {
    let mut cfg = test_config();
    cfg.bootstrap_admins = vec!["boss@example.com".into()];
    cfg.admin_require_mfa = true;
    let h = H::with(Arc::new(MemoryStore::new()), cfg);
    h.register_verified("boss@example.com").await;
    let pwd_only = h.login("boss@example.com").await["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let r = h.get_auth("/v1/admin/audit", &pwd_only).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{:?}", r.body);
    assert_eq!(r.body["error"], "mfa_required");
    // the service token is not a user login: exempt
    assert_eq!(
        h.get_auth("/v1/admin/audit", &h.service_token())
            .await
            .status,
        StatusCode::OK
    );

    let mut cfg = test_config();
    cfg.ip_requests_per_minute = 3;
    let h = H::with(Arc::new(MemoryStore::new()), cfg);
    let mut codes = Vec::new();
    for _ in 0..5 {
        codes.push(
            h.post_auth("/v1/2fa/totp/disable", "bogus", json!({"code": "1"}))
                .await
                .status,
        );
    }
    assert_eq!(
        codes
            .iter()
            .filter(|c| **c == StatusCode::TOO_MANY_REQUESTS)
            .count(),
        2,
        "{codes:?}"
    );
}
