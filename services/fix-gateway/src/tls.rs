//! TLS towards the LP (`[md.tls]` / `[trade.tls]`), built on `fix_session::tls`.

use std::sync::Arc;

use fix_session::tls::rustls::{
    self,
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer},
};

use crate::config::TlsEndpoint;

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("{0}: {1}")]
    Pem(String, rustls::pki_types::pem::Error),
    #[error("{0}: no certificates")]
    Empty(String),
    #[error("client_cert_file and client_key_file must be set together")]
    HalfClientAuth,
    #[error("tls config: {0}")]
    Rustls(#[from] rustls::Error),
}

/// Resolved TLS settings for one session.
#[derive(Clone)]
pub struct Tls {
    pub server_name: String,
    pub config: Arc<rustls::ClientConfig>,
}

/// Host part of `host:port` / `[v6]:port`.
fn host_of(addr: &str) -> &str {
    let h = addr.rsplit_once(':').map_or(addr, |(h, _)| h);
    h.trim_start_matches('[').trim_end_matches(']')
}

fn certs(path: &std::path::Path) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let name = path.display().to_string();
    let v = CertificateDer::pem_file_iter(path)
        .and_then(|it| it.collect::<Result<Vec<_>, _>>())
        .map_err(|e| TlsError::Pem(name.clone(), e))?;
    if v.is_empty() {
        return Err(TlsError::Empty(name));
    }
    Ok(v)
}

/// Builds the client config (ring provider, TLS 1.2/1.3).
pub fn build(ep: &TlsEndpoint, addr: &str) -> Result<Tls, TlsError> {
    let mut roots = rustls::RootCertStore::empty();
    match &ep.ca_file {
        Some(p) => {
            for c in certs(p)? {
                roots.add(c)?;
            }
        }
        None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots);
    let config = match (&ep.client_cert_file, &ep.client_key_file) {
        (None, None) => builder.with_no_client_auth(),
        (Some(c), Some(k)) => {
            let key = PrivateKeyDer::from_pem_file(k)
                .map_err(|e| TlsError::Pem(k.display().to_string(), e))?;
            builder.with_client_auth_cert(certs(c)?, key)?
        }
        _ => return Err(TlsError::HalfClientAuth),
    };
    Ok(Tls {
        server_name: ep
            .server_name
            .clone()
            .unwrap_or_else(|| host_of(addr).to_string()),
        config: Arc::new(config),
    })
}

#[cfg(test)]
mod tests {
    use super::host_of;

    #[test]
    fn host_parts() {
        assert_eq!(host_of("fix.lmax.com:443"), "fix.lmax.com");
        assert_eq!(host_of("[::1]:9880"), "::1");
        assert_eq!(host_of("noport"), "noport");
    }
}
