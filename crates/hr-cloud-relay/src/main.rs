mod relay;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use quinn::Endpoint;
use relay::ActiveConnection;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{error, info};

// ── Configuration ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Config {
    #[serde(default = "default_quic_port")]
    quic_port: u16,
    #[serde(default = "default_tcp_listen_port")]
    tcp_listen_port: u16,
    #[serde(default = "default_http_redirect_port")]
    http_redirect_port: u16,
    tls: TlsConfig,
}

#[derive(Deserialize)]
struct TlsConfig {
    ca_cert: String,
    server_cert: String,
    server_key: String,
}

fn default_quic_port() -> u16 {
    4443
}
fn default_tcp_listen_port() -> u16 {
    443
}
fn default_http_redirect_port() -> u16 {
    80
}

// ── Main ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .compact()
        .init();

    // Install rustls crypto provider
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // Parse CLI args for config path
    let config_path = parse_config_path();
    info!("Loading config from {}", config_path.display());

    // Load and parse config
    let config_str = std::fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
    let config: Config =
        toml::from_str(&config_str).with_context(|| "Failed to parse config file")?;

    // Load TLS certificates
    let server_cert_pem = std::fs::read(&config.tls.server_cert)
        .with_context(|| format!("Failed to read server cert: {}", config.tls.server_cert))?;
    let server_key_pem = std::fs::read(&config.tls.server_key)
        .with_context(|| format!("Failed to read server key: {}", config.tls.server_key))?;
    let ca_cert_pem = std::fs::read(&config.tls.ca_cert)
        .with_context(|| format!("Failed to read CA cert: {}", config.tls.ca_cert))?;

    // Build QUIC server config
    let server_config =
        hr_tunnel::quic::build_server_config(&server_cert_pem, &server_key_pem, &ca_cert_pem)?;

    // Create QUIC endpoint
    let quic_addr: SocketAddr = format!("[::]:{}", config.quic_port).parse()?;
    let endpoint = Endpoint::server(server_config, quic_addr)?;
    info!("QUIC endpoint listening on {}", quic_addr);

    // Shared active connection state
    let active_conn: ActiveConnection = Arc::new(RwLock::new(None));

    // Bind TCP relay listener
    let tcp_addr: SocketAddr = format!("[::]:{}", config.tcp_listen_port).parse()?;
    let tcp_listener = TcpListener::bind(tcp_addr)
        .await
        .with_context(|| format!("Failed to bind TCP relay on {}", tcp_addr))?;

    // Spawn TCP relay
    let relay_conn = active_conn.clone();
    tokio::spawn(async move {
        if let Err(e) = relay::run_tcp_relay(tcp_listener, relay_conn).await {
            error!("TCP relay error: {}", e);
        }
    });

    // Spawn HTTP redirect server
    let http_port = config.http_redirect_port;
    tokio::spawn(async move {
        if let Err(e) = relay::run_http_redirect(http_port).await {
            error!("HTTP redirect error: {}", e);
        }
    });

    info!("hr-cloud-relay started successfully");

    // Main loop: accept QUIC connections + handle shutdown
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else {
                    info!("QUIC endpoint closed");
                    break;
                };

                let active = active_conn.clone();
                tokio::spawn(async move {
                    match incoming.await {
                        Ok(connection) => {
                            let remote = connection.remote_address();
                            info!("Tunnel connection established from {}", remote);

                            // Replace the active connection
                            *active.write().await = Some(connection.clone());

                            // Spawn control stream handler
                            let ctrl_conn = connection.clone();
                            tokio::spawn(async move {
                                relay::handle_control_stream(&ctrl_conn).await;
                            });

                            // Monitor connection lifetime
                            let conn_id = connection.stable_id();
                            let active_for_cleanup = active.clone();
                            let err = connection.closed().await;
                            info!("Tunnel connection from {} closed: {}", remote, err);

                            // Clear active connection if it's still this one
                            let mut guard = active_for_cleanup.write().await;
                            if let Some(ref current) = *guard {
                                if current.stable_id() == conn_id {
                                    *guard = None;
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to accept QUIC connection: {}", e);
                        }
                    }
                });
            }
            _ = &mut shutdown => {
                info!("Shutdown signal received, stopping...");
                endpoint.close(0u32.into(), b"shutdown");
                break;
            }
        }
    }

    info!("hr-cloud-relay stopped");
    Ok(())
}

fn parse_config_path() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "--config" {
            if let Some(path) = args.get(i + 1) {
                return PathBuf::from(path);
            }
        }
        if let Some(path) = args[i].strip_prefix("--config=") {
            return PathBuf::from(path);
        }
    }
    PathBuf::from("/etc/hr-cloud-relay/config.toml")
}
