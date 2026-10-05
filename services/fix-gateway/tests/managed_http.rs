//! Admin endpoint: PUT/GET /config with bearer token, passwords never read back.

use std::sync::{Arc, RwLock};

use fix_gateway::managed::Managed;
use fix_gateway::status_http::{spawn_with_admin, Admin};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOKEN: &str = "0123456789abcdef-test";

async fn http(
    addr: std::net::SocketAddr,
    method: &str,
    token: Option<&str>,
    body: &str,
) -> (u16, String) {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let auth = token
        .map(|t| format!("authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "{method} /config HTTP/1.1\r\nhost: x\r\n{auth}content-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let code = out[9..12].parse().unwrap();
    let body = out.split_once("\r\n\r\n").unwrap().1.to_string();
    (code, body)
}

#[tokio::test]
async fn put_and_get_config() {
    let dir = tempfile::tempdir().unwrap();
    let managed = Arc::new(Managed::open(dir.path().join("lp.json")).unwrap());
    let admin = Admin {
        token: TOKEN.into(),
        managed: managed.clone(),
    };
    let addr = spawn_with_admin("127.0.0.1:0", Arc::new(RwLock::new(vec![])), Some(admin))
        .await
        .unwrap();

    let mut cfg =
        fix_gateway::GatewayConfig::from_toml(include_str!("../config/default.toml")).unwrap();
    cfg.md.password = Some("hunter2-secret".into());
    let body = serde_json::to_string(&cfg).unwrap();

    assert_eq!(http(addr, "PUT", None, &body).await.0, 401);
    assert_eq!(
        http(addr, "PUT", Some("wrong-token-xxxxxxx"), &body)
            .await
            .0,
        401
    );
    assert_eq!(http(addr, "PUT", Some(TOKEN), "{\"nope\":1}").await.0, 400);

    let (code, out) = http(addr, "PUT", Some(TOKEN), &body).await;
    assert_eq!(code, 200, "{out}");
    assert!(!out.contains("hunter2"));
    let (code, out) = http(addr, "GET", Some(TOKEN), "").await;
    assert_eq!(code, 200);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["md"]["password_set"], true);
    assert!(v["md"]["password"].is_null());
    assert_eq!(
        managed.current().unwrap().md.password.as_deref(),
        Some("hunter2-secret")
    );
}
