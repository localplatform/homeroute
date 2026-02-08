use anyhow::{Context, Result};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::Arc;

/// Build a quinn::ServerConfig for the VPS side (accepts tunnel connections).
/// Uses the server cert and requires client certs signed by our CA.
pub fn build_server_config(
    server_cert_pem: &[u8],
    server_key_pem: &[u8],
    ca_cert_pem: &[u8],
) -> Result<quinn::ServerConfig> {
    let certs = load_certs(server_cert_pem)?;
    let key = load_private_key(server_key_pem)?;

    // Build root store with our CA for client verification
    let mut root_store = rustls::RootCertStore::empty();
    let ca_certs = load_certs(ca_cert_pem)?;
    for cert in &ca_certs {
        root_store
            .add(cert.clone())
            .context("Failed to add CA cert to root store")?;
    }

    let client_verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(root_store))
        .build()
        .context("Failed to build client verifier")?;

    let server_crypto = rustls::ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(certs, key)
        .context("Failed to build server TLS config")?;

    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
            .context("Failed to create QUIC server config")?,
    ));

    // Tune transport for relay workload
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_bidi_streams(4096u32.into());
    transport.max_concurrent_uni_streams(256u32.into());
    transport.keep_alive_interval(Some(std::time::Duration::from_secs(10)));
    server_config.transport_config(Arc::new(transport));

    Ok(server_config)
}

/// Build a quinn::ClientConfig for the on-prem side (connects to VPS).
/// Uses the client cert and trusts only our CA.
pub fn build_client_config(
    client_cert_pem: &[u8],
    client_key_pem: &[u8],
    ca_cert_pem: &[u8],
) -> Result<quinn::ClientConfig> {
    let certs = load_certs(client_cert_pem)?;
    let key = load_private_key(client_key_pem)?;

    // Trust only our CA
    let mut root_store = rustls::RootCertStore::empty();
    let ca_certs = load_certs(ca_cert_pem)?;
    for cert in &ca_certs {
        root_store
            .add(cert.clone())
            .context("Failed to add CA cert to root store")?;
    }

    let client_crypto = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_client_auth_cert(certs, key)
        .context("Failed to build client TLS config")?;

    let mut client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)
            .context("Failed to create QUIC client config")?,
    ));

    // Tune transport for relay workload
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_bidi_streams(4096u32.into());
    transport.max_concurrent_uni_streams(256u32.into());
    transport.keep_alive_interval(Some(std::time::Duration::from_secs(10)));
    client_config.transport_config(Arc::new(transport));

    Ok(client_config)
}

fn load_certs(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>> {
    let mut reader = std::io::BufReader::new(pem);
    let certs: Vec<_> = rustls_pemfile::certs(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to parse PEM certificates")?;
    anyhow::ensure!(!certs.is_empty(), "No certificates found in PEM");
    Ok(certs)
}

fn load_private_key(pem: &[u8]) -> Result<PrivateKeyDer<'static>> {
    let mut reader = std::io::BufReader::new(pem);
    // Try PKCS#8 first, then EC, then RSA
    let keys: Vec<_> = rustls_pemfile::pkcs8_private_keys(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to parse private keys")?;
    if let Some(key) = keys.into_iter().next() {
        return Ok(PrivateKeyDer::Pkcs8(key));
    }

    // Retry with EC keys
    let mut reader = std::io::BufReader::new(pem);
    let keys: Vec<_> = rustls_pemfile::ec_private_keys(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to parse EC keys")?;
    if let Some(key) = keys.into_iter().next() {
        return Ok(PrivateKeyDer::Sec1(key));
    }

    anyhow::bail!("No private key found in PEM")
}
