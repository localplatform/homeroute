mod config;
mod tls;
mod proxy;
mod auth;
mod logging;

use config::ProxyConfig;
use proxy::ProxyState;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{info, error, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::Request;
use hyper_util::rt::TokioIo;
use axum::response::IntoResponse;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Install rustls crypto provider
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rust_proxy=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Rust Reverse Proxy...");

    // Load configuration
    let config_path = std::env::var("PROXY_CONFIG_PATH")
        .unwrap_or_else(|_| "/var/lib/server-dashboard/rust-proxy-config.json".to_string());
    let config_path = PathBuf::from(&config_path);

    let config = load_config(&config_path)?;

    info!("Configuration loaded from: {:?}", config_path);
    info!("Base domain: {}", config.base_domain);
    info!("HTTPS port: {}", config.https_port);
    info!("Active routes: {}", config.active_routes().len());

    // Initialize TLS manager and load certificates
    let tls_manager = Arc::new(tls::TlsManager::new(config.ca_storage_path.clone()));

    info!("Loading TLS certificates...");
    for route in config.active_routes() {
        if let Some(cert_id) = &route.cert_id {
            match tls_manager.load_certificate(&route.domain, cert_id) {
                Ok(_) => info!("  Loaded certificate for: {}", route.domain),
                Err(e) => error!("  Failed to load certificate for {}: {}", route.domain, e),
            }
        } else {
            warn!("  No certificate configured for: {}", route.domain);
        }
    }

    // Build TLS server config with SNI resolver
    let tls_config = tls_manager.build_server_config()?;
    let tls_acceptor = TlsAcceptor::from(tls_config);

    // Initialize proxy state
    let proxy_state = Arc::new(ProxyState::new(config.clone()));

    // Clone for SIGHUP handler
    let tls_manager_reload = tls_manager.clone();
    let config_path_reload = config_path.clone();
    let proxy_state_reload = proxy_state.clone();

    // Spawn SIGHUP handler for hot-reload
    tokio::spawn(async move {
        use signal_hook::consts::SIGHUP;
        use signal_hook_tokio::Signals;
        use tokio_stream::StreamExt;

        let mut signals = match Signals::new(&[SIGHUP]) {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to register SIGHUP handler: {}", e);
                return;
            }
        };

        while let Some(sig) = signals.next().await {
            if sig == SIGHUP {
                info!("Received SIGHUP - reloading configuration...");
                match load_config(&config_path_reload) {
                    Ok(new_config) => {
                        // Reload TLS certificates
                        match tls_manager_reload.reload_certificates(&new_config.routes) {
                            Ok(_) => {},
                            Err(e) => error!("Failed to reload certificates: {}", e),
                        }
                        // Reload proxy routes
                        let route_count = new_config.active_routes().len();
                        proxy_state_reload.reload_config(new_config);
                        info!("Configuration reloaded successfully ({} active routes)", route_count);
                    }
                    Err(e) => error!("Failed to reload config: {}", e),
                }
            }
        }
    });

    // Spawn HTTP→HTTPS redirect server on port 80
    let http_port = config.http_port;
    tokio::spawn(async move {
        let http_addr = SocketAddr::from(([0, 0, 0, 0], http_port));
        let listener = match TcpListener::bind(http_addr).await {
            Ok(l) => l,
            Err(e) => {
                error!("Failed to bind HTTP redirect listener on port {}: {}", http_port, e);
                return;
            }
        };
        info!("HTTP redirect server listening on {}", http_addr);

        loop {
            let (stream, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    error!("Failed to accept HTTP connection: {}", e);
                    continue;
                }
            };

            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let service = service_fn(|req: Request<Incoming>| async move {
                    let host = req.headers()
                        .get("host")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("localhost")
                        .split(':').next().unwrap_or("localhost");
                    let path = req.uri().path_and_query()
                        .map(|pq| pq.as_str())
                        .unwrap_or("/");
                    let location = format!("https://{}{}", host, path);

                    Ok::<_, std::convert::Infallible>(
                        hyper::Response::builder()
                            .status(301)
                            .header("Location", location)
                            .body(axum::body::Body::empty())
                            .unwrap()
                    )
                });

                if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                    let msg = e.to_string();
                    if !msg.contains("connection closed") && !msg.contains("not connected") {
                        error!("HTTP redirect error: {}", e);
                    }
                }
            });
        }
    });

    // Start HTTPS server with TLS
    let https_addr = SocketAddr::from(([0, 0, 0, 0], config.https_port));
    let listener = TcpListener::bind(https_addr).await?;
    info!("HTTPS server listening on {}", https_addr);

    loop {
        let (tcp_stream, remote_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                error!("Failed to accept TCP connection: {}", e);
                continue;
            }
        };

        let tls_acceptor = tls_acceptor.clone();
        let proxy_state = proxy_state.clone();
        let client_ip = remote_addr.ip();

        tokio::spawn(async move {
            // TLS handshake
            let tls_stream = match tls_acceptor.accept(tcp_stream).await {
                Ok(s) => s,
                Err(e) => {
                    warn!("TLS handshake failed from {}: {}", remote_addr, e);
                    return;
                }
            };

            let io = TokioIo::new(tls_stream);

            // Serve HTTP/1.1 over TLS
            // Convert Incoming body to axum Body for our handler
            let service = service_fn(move |req: Request<Incoming>| {
                let state = proxy_state.clone();
                async move {
                    let (parts, body) = req.into_parts();
                    let body = axum::body::Body::new(body);
                    let req = axum::extract::Request::from_parts(parts, body);
                    // Convert ProxyError to Response so hyper always gets Ok
                    let response = match proxy::proxy_handler(state, client_ip, req).await {
                        Ok(resp) => resp,
                        Err(e) => e.into_response(),
                    };
                    Ok::<_, std::convert::Infallible>(response)
                }
            });

            let conn = http1::Builder::new()
                .preserve_header_case(true)
                .title_case_headers(true)
                .serve_connection(io, service)
                .with_upgrades();

            if let Err(e) = conn.await {
                let msg = e.to_string();
                if !msg.contains("connection closed") && !msg.contains("not connected") {
                    error!("HTTP error from {}: {}", remote_addr, e);
                }
            }
        });
    }
}

fn load_config(path: &PathBuf) -> anyhow::Result<ProxyConfig> {
    if path.exists() {
        ProxyConfig::load_from_file(path)
    } else {
        info!("Config file not found, creating default configuration");
        let default_config = ProxyConfig {
            http_port: 80,
            https_port: 443,
            base_domain: "mynetwk.biz".to_string(),
            tls_mode: "local-ca".to_string(),
            ca_storage_path: PathBuf::from("/var/lib/server-dashboard/ca"),
            auth_service_url: "http://localhost:4000".to_string(),
            routes: vec![],
            access_log_path: None,
            local_networks: vec![
                "192.168.0.0/16".to_string(),
                "10.0.0.0/8".to_string(),
                "172.16.0.0/12".to_string(),
                "127.0.0.0/8".to_string(),
            ],
        };
        default_config.save_to_file(path)?;
        Ok(default_config)
    }
}
