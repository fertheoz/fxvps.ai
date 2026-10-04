//! Optional TLS (feature `tls`) via rustls. LP connections in production typically
//! run over TLS or a cross-connect (LMAX: [DOĞRULA], see docs/02).

use std::sync::Arc;

use tokio::net::TcpStream;
pub use tokio_rustls::rustls;
use tokio_rustls::{client::TlsStream, TlsConnector};

/// Connects to `addr` and performs a TLS handshake for `server_name` using the
/// caller-provided client configuration (root store, client certs, ...).
pub async fn connect_tls(
    addr: &str,
    server_name: &str,
    config: Arc<rustls::ClientConfig>,
) -> std::io::Result<TlsStream<TcpStream>> {
    let tcp = TcpStream::connect(addr).await?;
    tcp.set_nodelay(true)?;
    let name = rustls::pki_types::ServerName::try_from(server_name.to_owned())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    TlsConnector::from(config).connect(name, tcp).await
}
