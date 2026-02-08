use anyhow::{Context, Result};
use rcgen::{
    CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
    SanType, PKCS_ECDSA_P256_SHA256,
};
use std::net::IpAddr;
use std::time::Duration;

/// Generated tunnel certificate material (all PEM-encoded).
pub struct TunnelCerts {
    pub ca_cert_pem: String,
    pub ca_key_pem: String,
    pub server_cert_pem: String,
    pub server_key_pem: String,
    pub client_cert_pem: String,
    pub client_key_pem: String,
}

/// Generate a self-signed CA and issue server + client certificates for the QUIC tunnel.
///
/// - The server cert has the VPS hostname (or IP) as SAN.
/// - The client cert has "homeroute-onprem" as CN.
/// - All certs valid for 10 years.
pub fn generate_tunnel_certs(vps_host: &str) -> Result<TunnelCerts> {
    let validity = Duration::from_secs(10 * 365 * 24 * 3600);

    // ── CA ────────────────────────────────────────────────────────────
    let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .context("Failed to generate CA key pair")?;

    let mut ca_params = CertificateParams::new(Vec::<String>::new())
        .context("Failed to create CA params")?;
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "HomeRoute Tunnel CA");
    ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params.not_before = time::OffsetDateTime::now_utc();
    ca_params.not_after = time::OffsetDateTime::now_utc() + validity;

    let ca_cert = ca_params
        .self_signed(&ca_key)
        .context("Failed to self-sign CA cert")?;

    // ── Server cert ──────────────────────────────────────────────────
    let server_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .context("Failed to generate server key pair")?;

    let server_sans = build_sans(vps_host);
    let mut server_params = CertificateParams::new(Vec::<String>::new())
        .context("Failed to create server params")?;
    server_params
        .distinguished_name
        .push(DnType::CommonName, vps_host);
    server_params.subject_alt_names = server_sans;
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    server_params.not_before = time::OffsetDateTime::now_utc();
    server_params.not_after = time::OffsetDateTime::now_utc() + validity;

    let server_cert = server_params
        .signed_by(&server_key, &ca_cert, &ca_key)
        .context("Failed to sign server cert")?;

    // ── Client cert ──────────────────────────────────────────────────
    let client_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .context("Failed to generate client key pair")?;

    let mut client_params = CertificateParams::new(Vec::<String>::new())
        .context("Failed to create client params")?;
    client_params
        .distinguished_name
        .push(DnType::CommonName, "homeroute-onprem");
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    client_params.not_before = time::OffsetDateTime::now_utc();
    client_params.not_after = time::OffsetDateTime::now_utc() + validity;

    let client_cert = client_params
        .signed_by(&client_key, &ca_cert, &ca_key)
        .context("Failed to sign client cert")?;

    Ok(TunnelCerts {
        ca_cert_pem: ca_cert.pem(),
        ca_key_pem: ca_key.serialize_pem(),
        server_cert_pem: server_cert.pem(),
        server_key_pem: server_key.serialize_pem(),
        client_cert_pem: client_cert.pem(),
        client_key_pem: client_key.serialize_pem(),
    })
}

/// Build SAN entries: if `host` parses as an IP, use IpAddress SAN; otherwise DNS.
fn build_sans(host: &str) -> Vec<SanType> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        vec![SanType::IpAddress(ip)]
    } else {
        vec![SanType::DnsName(host.try_into().expect("valid DNS name"))]
    }
}
