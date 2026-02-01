mod supervisor;

use hr_adblock::AdblockEngine;
use hr_auth::AuthService;
use hr_ca::{CaConfig, CertificateAuthority};
use hr_common::config::EnvConfig;
use hr_common::events::EventBus;
use hr_dns::DnsState;
use hr_proxy::{ProxyConfig, ProxyState, TlsManager};
use signal_hook::consts::SIGHUP;
use signal_hook_tokio::Signals;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use supervisor::{spawn_supervised, ServicePriority};
use tokio::sync::RwLock;
use tokio_stream::StreamExt;
use tracing::{error, info, warn};

/// Combined config from dns-dhcp-config.json (matches the original file layout)
#[derive(serde::Deserialize, Default)]
struct DnsDhcpConfig {
    #[serde(default)]
    dns: hr_dns::DnsConfig,
    #[serde(default)]
    dhcp: hr_dhcp::DhcpConfig,
    #[serde(default)]
    ipv6: hr_ipv6::Ipv6Config,
    #[serde(default)]
    adblock: hr_adblock::config::AdblockConfig,
}

impl DnsDhcpConfig {
    fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            info!("No config file at {}, using defaults", path.display());
            Ok(Self::default())
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,homeroute=debug".parse().unwrap()),
        )
        .init();

    info!("HomeRoute starting...");

    // Install rustls crypto provider
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // Load environment config
    let env = EnvConfig::load(None);
    info!("Base domain: {}", env.base_domain);

    // Initialize event bus
    let events = Arc::new(EventBus::new());

    // Initialize auth service
    let auth = AuthService::new(&env.auth_data_dir, &env.base_domain)?;
    auth.start_cleanup_task();
    info!("Auth service initialized");

    // Initialize CA
    let ca_config = CaConfig {
        storage_path: env.ca_storage_path.to_string_lossy().to_string(),
        ..CaConfig::default()
    };
    let ca = Arc::new(CertificateAuthority::new(ca_config));
    ca.init().await?;
    info!(
        "Certificate Authority initialized ({})",
        if ca.is_initialized() {
            "loaded existing"
        } else {
            "generated new"
        }
    );

    // ── Load DNS/DHCP/IPv6/Adblock config ──────────────────────────────

    let dns_dhcp_config = DnsDhcpConfig::load(&env.dns_dhcp_config_path)?;

    info!(
        "Config loaded: DNS port {}, DHCP {}, Adblock {}, IPv6 {}",
        dns_dhcp_config.dns.port,
        if dns_dhcp_config.dhcp.enabled { "enabled" } else { "disabled" },
        if dns_dhcp_config.adblock.enabled { "enabled" } else { "disabled" },
        if dns_dhcp_config.ipv6.enabled { "enabled" } else { "disabled" },
    );

    // ── Initialize adblock engine ──────────────────────────────────────

    let mut adblock_engine = AdblockEngine::new();
    adblock_engine.set_whitelist(dns_dhcp_config.adblock.whitelist.clone());

    if dns_dhcp_config.adblock.enabled {
        let cache_path = PathBuf::from(&dns_dhcp_config.adblock.data_dir).join("domains.json");
        match hr_adblock::sources::load_cache(&cache_path) {
            Ok(domains) => {
                info!("Loaded {} blocked domains from cache", domains.len());
                adblock_engine.set_blocked(domains);
            }
            Err(_) => {
                info!("No adblock cache found, will download on startup");
            }
        }
    }

    let adblock = Arc::new(RwLock::new(adblock_engine));

    // ── Initialize DHCP state ──────────────────────────────────────────

    let server_ip: Ipv4Addr = dns_dhcp_config
        .dhcp
        .gateway
        .parse()
        .unwrap_or(Ipv4Addr::UNSPECIFIED);

    let mut lease_store = hr_dhcp::LeaseStore::new(&dns_dhcp_config.dhcp.lease_file);
    if let Err(e) = lease_store.load_from_file() {
        warn!("Failed to load lease file: {}", e);
    }

    let dhcp_state: hr_dhcp::SharedDhcpState = Arc::new(RwLock::new(hr_dhcp::DhcpState {
        config: dns_dhcp_config.dhcp.clone(),
        lease_store,
        server_ip,
    }));

    // Separate LeaseStore for DNS resolver (synced from DHCP state every 10s).
    // DhcpState owns its LeaseStore directly; DnsState needs Arc<RwLock<LeaseStore>>.
    // A background sync task keeps them in sync.
    let lease_store_for_dns: Arc<RwLock<hr_dhcp::LeaseStore>> = {
        let mut shared_lease_store = hr_dhcp::LeaseStore::new(&dns_dhcp_config.dhcp.lease_file);
        if let Err(e) = shared_lease_store.load_from_file() {
            warn!("Failed to load lease file for DNS: {}", e);
        }
        Arc::new(RwLock::new(shared_lease_store))
    };

    // ── Initialize DNS state ───────────────────────────────────────────

    let dns_cache = hr_dns::cache::DnsCache::new(dns_dhcp_config.dns.cache_size);

    let upstream = hr_dns::upstream::UpstreamForwarder::new(
        dns_dhcp_config.dns.upstream_servers.clone(),
        dns_dhcp_config.dns.upstream_timeout_ms,
    );

    let query_logger = if !dns_dhcp_config.dns.query_log_path.is_empty() {
        Some(hr_dns::logging::QueryLogger::new(
            &dns_dhcp_config.dns.query_log_path,
        ))
    } else {
        None
    };

    let dns_state: hr_dns::SharedDnsState = Arc::new(RwLock::new(DnsState {
        config: dns_dhcp_config.dns.clone(),
        dns_cache,
        upstream,
        query_logger,
        adblock: adblock.clone(),
        lease_store: lease_store_for_dns.clone(),
        adblock_enabled: dns_dhcp_config.adblock.enabled,
        adblock_block_response: dns_dhcp_config.adblock.block_response.clone(),
        dns_events: Some(events.dns_traffic.clone()),
    }));

    // ── Initialize proxy ───────────────────────────────────────────────

    let proxy_config_path = env.proxy_config_path.clone();
    let proxy_config = if proxy_config_path.exists() {
        ProxyConfig::load_from_file(&proxy_config_path)?
    } else {
        ProxyConfig {
            base_domain: env.base_domain.clone(),
            ca_storage_path: env.ca_storage_path.clone(),
            ..serde_json::from_str("{}")?
        }
    };

    let tls_manager = TlsManager::new(proxy_config.ca_storage_path.clone());

    // Load TLS certificates for each active route
    for route in proxy_config.active_routes() {
        if let Some(ref cert_id) = route.cert_id {
            if let Err(e) = tls_manager.load_certificate(&route.domain, cert_id) {
                error!("Failed to load TLS cert for {}: {}", route.domain, e);
            }
        }
    }

    let tls_config = tls_manager.build_server_config()?;

    let proxy_state = Arc::new(
        ProxyState::new(proxy_config.clone())
            .with_auth(auth.clone())
            .with_events(events.http_traffic.clone()),
    );

    let https_port = proxy_config.https_port;
    let http_port = proxy_config.http_port;

    info!(
        "Loaded {} TLS certificates for {} active routes",
        tls_manager.loaded_domains().len(),
        proxy_config.active_routes().len()
    );

    // ── Store shared refs for SIGHUP reload ────────────────────────────

    let dns_state_reload = dns_state.clone();
    let proxy_state_reload = proxy_state.clone();
    let adblock_reload = adblock.clone();
    let dns_dhcp_config_path = env.dns_dhcp_config_path.clone();
    let proxy_config_path_reload = env.proxy_config_path.clone();
    let tls_manager = Arc::new(tls_manager);
    let tls_manager_reload = tls_manager.clone();

    // ── Spawn supervised services ──────────────────────────────────────

    info!("Starting supervised services...");

    // DNS UDP server (Critical)
    for addr_str in &dns_dhcp_config.dns.listen_addresses {
        let addr: SocketAddr = format!("{}:{}", addr_str, dns_dhcp_config.dns.port).parse()?;

        let dns_state_c = dns_state.clone();
        spawn_supervised("dns-udp", ServicePriority::Critical, move || {
            let state = dns_state_c.clone();
            let addr = addr;
            async move { hr_dns::server::run_udp_server(addr, state).await }
        });

        let dns_state_c = dns_state.clone();
        spawn_supervised("dns-tcp", ServicePriority::Critical, move || {
            let state = dns_state_c.clone();
            let addr = addr;
            async move { hr_dns::server::run_tcp_server(addr, state).await }
        });
    }

    // DHCP server (Critical)
    if dns_dhcp_config.dhcp.enabled {
        let dhcp_state_c = dhcp_state.clone();
        spawn_supervised("dhcp", ServicePriority::Critical, move || {
            let state = dhcp_state_c.clone();
            async move { hr_dhcp::server::run_dhcp_server(state).await }
        });
    }

    // HTTPS proxy (Critical)
    {
        let proxy_state_c = proxy_state.clone();
        let tls_config_c = tls_config.clone();
        spawn_supervised("proxy-https", ServicePriority::Critical, move || {
            let proxy_state = proxy_state_c.clone();
            let tls_config = tls_config_c.clone();
            let port = https_port;
            async move { run_https_server(proxy_state, tls_config, port).await }
        });
    }

    // HTTP redirect (Critical)
    {
        let base_domain = env.base_domain.clone();
        spawn_supervised("proxy-http", ServicePriority::Critical, move || {
            let base_domain = base_domain.clone();
            let port = http_port;
            async move { run_http_redirect(port, &base_domain).await }
        });
    }

    // IPv6 RA sender (Important)
    if dns_dhcp_config.ipv6.enabled && dns_dhcp_config.ipv6.ra_enabled {
        let ipv6_config = dns_dhcp_config.ipv6.clone();
        spawn_supervised("ipv6-ra", ServicePriority::Important, move || {
            let config = ipv6_config.clone();
            async move { hr_ipv6::ra::run_ra_sender(config).await }
        });
    }

    // DHCPv6 (Important)
    if dns_dhcp_config.ipv6.enabled && dns_dhcp_config.ipv6.dhcpv6_enabled {
        let ipv6_config = dns_dhcp_config.ipv6.clone();
        spawn_supervised("dhcpv6", ServicePriority::Important, move || {
            let config = ipv6_config.clone();
            async move { hr_ipv6::dhcpv6::run_dhcpv6_server(config).await }
        });
    }

    // ── Analytics (Background) ─────────────────────────────────────────

    let analytics_db_path = format!("{}/analytics.db", env.data_dir.display());
    let analytics_store = Arc::new(
        hr_analytics::store::AnalyticsStore::open(&analytics_db_path)
            .expect("Failed to open analytics database"),
    );
    info!("Analytics store opened at {}", analytics_db_path);

    // HTTP traffic capture (from proxy broadcast channel)
    {
        let store = analytics_store.clone();
        let rx = events.http_traffic.subscribe();
        let leases = lease_store_for_dns.clone();
        tokio::spawn(async move {
            hr_analytics::capture::run_http_capture(store, rx, leases).await;
        });
    }

    // DNS traffic capture (from DNS broadcast channel)
    {
        let store = analytics_store.clone();
        let rx = events.dns_traffic.subscribe();
        let leases = lease_store_for_dns.clone();
        tokio::spawn(async move {
            hr_analytics::capture::run_dns_capture(store, rx, leases).await;
        });
    }

    // Hourly aggregation (every 5 minutes)
    {
        let store = analytics_store.clone();
        tokio::spawn(async move {
            hr_analytics::aggregation::run_hourly_aggregation(store).await;
        });
    }

    // Daily aggregation + cleanup (at 00:30 UTC)
    {
        let store = analytics_store.clone();
        tokio::spawn(async move {
            hr_analytics::aggregation::run_daily_aggregation(store).await;
        });
    }

    // ── Management API (Important) ────────────────────────────────────

    let api_state = hr_api::state::ApiState {
        auth: auth.clone(),
        ca: ca.clone(),
        proxy: proxy_state.clone(),
        dns: dns_state.clone(),
        dhcp: dhcp_state.clone(),
        adblock: adblock.clone(),
        events: events.clone(),
        env: Arc::new(env.clone()),
        analytics: analytics_store.clone(),
        dns_dhcp_config_path: env.dns_dhcp_config_path.clone(),
        proxy_config_path: env.proxy_config_path.clone(),
        reverseproxy_config_path: env.reverseproxy_config_path.clone(),
    };

    let api_router = hr_api::build_router(api_state);
    let api_port = env.api_port;

    spawn_supervised("api", ServicePriority::Important, move || {
        let router = api_router.clone();
        let port = api_port;
        async move {
            let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
            let listener = tokio::net::TcpListener::bind(addr).await?;
            info!("Management API listening on {}", addr);
            axum::serve(listener, router).await?;
            Ok(())
        }
    });

    // ── Background tasks ───────────────────────────────────────────────

    // Lease persistence (save every 60s)
    {
        let dhcp_state_c = dhcp_state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                let s = dhcp_state_c.read().await;
                if let Err(e) = s.lease_store.save_to_file() {
                    warn!("Failed to save lease file: {}", e);
                }
            }
        });
    }

    // Sync DHCP leases → DNS lease store (every 10s)
    {
        let dhcp_state_c = dhcp_state.clone();
        let lease_store_dns = lease_store_for_dns.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                // Copy all leases from DHCP state to DNS lease store
                let dhcp_read = dhcp_state_c.read().await;
                let all_leases: Vec<_> = dhcp_read
                    .lease_store
                    .all_leases()
                    .into_iter()
                    .cloned()
                    .collect();
                drop(dhcp_read);

                let mut dns_ls = lease_store_dns.write().await;
                // Rebuild: clear and re-add all (cheap, ~100 entries max)
                // We can't clear easily, so just add/update each lease
                for lease in all_leases {
                    dns_ls.add_lease(lease);
                }
            }
        });
    }

    // DNS cache purge (every 30s)
    {
        let dns_state_c = dns_state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let s = dns_state_c.read().await;
                let purged = s.dns_cache.purge_expired().await;
                if purged > 0 {
                    info!("Purged {} expired DNS cache entries", purged);
                }
            }
        });
    }

    // Adblock initial download + auto-update
    if dns_dhcp_config.adblock.enabled {
        let adblock_c = adblock.clone();
        let sources = dns_dhcp_config.adblock.sources.clone();
        let data_dir = dns_dhcp_config.adblock.data_dir.clone();
        let dns_state_c = dns_state.clone();
        tokio::spawn(async move {
            // Initial download after 5s delay
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            info!("Starting initial adblock list download...");
            do_adblock_update(&adblock_c, &sources, &data_dir, &dns_state_c).await;
        });

        if dns_dhcp_config.adblock.auto_update_hours > 0 {
            let adblock_c = adblock.clone();
            let sources = dns_dhcp_config.adblock.sources.clone();
            let data_dir = dns_dhcp_config.adblock.data_dir.clone();
            let dns_state_c = dns_state.clone();
            let hours = dns_dhcp_config.adblock.auto_update_hours;
            tokio::spawn(async move {
                let interval = std::time::Duration::from_secs(hours * 3600);
                loop {
                    tokio::time::sleep(interval).await;
                    info!("Running scheduled adblock update...");
                    do_adblock_update(&adblock_c, &sources, &data_dir, &dns_state_c).await;
                }
            });
        }
    }

    // ── Server monitoring & scheduler (Background) ────────────────────

    // Server monitoring (ping all servers every 30s)
    {
        let server_events = Arc::new(events.server_status.clone());
        tokio::spawn(async move {
            hr_servers::monitoring::run_monitoring(server_events).await;
        });
    }

    // WoL schedule executor (check cron schedules every 30s)
    tokio::spawn(async move {
        hr_servers::scheduler::run_scheduler().await;
    });

    // ── SIGHUP handler ─────────────────────────────────────────────────

    tokio::spawn(async move {
        if let Err(e) = handle_sighup(
            dns_dhcp_config_path,
            proxy_config_path_reload,
            dns_state_reload,
            proxy_state_reload,
            adblock_reload,
            tls_manager_reload,
        )
        .await
        {
            error!("SIGHUP handler error: {}", e);
        }
    });

    // ── Ready ──────────────────────────────────────────────────────────

    info!("HomeRoute started successfully");
    info!("  Auth: OK");
    info!(
        "  CA: OK ({} certificates)",
        ca.list_certificates().unwrap_or_default().len()
    );
    info!("  Events: OK (broadcast bus)");
    info!(
        "  DNS: listening on port {}",
        dns_dhcp_config.dns.port
    );
    info!(
        "  DHCP: {}",
        if dns_dhcp_config.dhcp.enabled {
            "listening on port 67"
        } else {
            "disabled"
        }
    );
    info!("  Proxy: HTTPS:{} HTTP:{}", https_port, http_port);
    info!("  API: listening on port {}", api_port);
    info!(
        "  IPv6: {}",
        if dns_dhcp_config.ipv6.enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "  Adblock: {} domains blocked",
        adblock.read().await.domain_count()
    );
    info!("  Servers: monitoring every 30s, scheduler active");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    info!("Shutting down...");

    // Save leases on shutdown
    {
        let s = dhcp_state.read().await;
        if let Err(e) = s.lease_store.save_to_file() {
            error!("Failed to save leases on shutdown: {}", e);
        } else {
            info!("Leases saved successfully");
        }
    }

    Ok(())
}

// ── HTTPS server ───────────────────────────────────────────────────────

async fn run_https_server(
    proxy_state: Arc<ProxyState>,
    tls_config: Arc<rustls::ServerConfig>,
    port: u16,
) -> anyhow::Result<()> {
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use tokio_rustls::TlsAcceptor;

    let addr: SocketAddr = format!("[::]:{}", port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let acceptor = TlsAcceptor::from(tls_config);

    info!("HTTPS proxy listening on {}", addr);

    loop {
        let (tcp_stream, remote_addr) = match listener.accept().await {
            Ok(r) => r,
            Err(e) => {
                warn!("TCP accept error: {}", e);
                continue;
            }
        };

        let acceptor = acceptor.clone();
        let proxy_state = proxy_state.clone();
        let client_ip = remote_addr.ip();

        tokio::spawn(async move {
            let tls_stream = match acceptor.accept(tcp_stream).await {
                Ok(s) => s,
                Err(e) => {
                    // TLS handshake failures are common (scanners, invalid SNI)
                    tracing::debug!("TLS handshake failed from {}: {}", remote_addr, e);
                    return;
                }
            };

            let io = TokioIo::new(tls_stream);
            let service = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                let state = proxy_state.clone();
                async move {
                    // Convert Incoming → axum Body
                    let (parts, body) = req.into_parts();
                    let req = axum::extract::Request::from_parts(parts, axum::body::Body::new(body));
                    let resp = hr_proxy::proxy_handler(state, client_ip, req).await;
                    Ok::<_, std::convert::Infallible>(axum::response::IntoResponse::into_response(resp))
                }
            });

            if let Err(e) = http1::Builder::new()
                .preserve_header_case(true)
                .title_case_headers(true)
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                let msg = e.to_string();
                if !msg.contains("connection closed")
                    && !msg.contains("not connected")
                    && !msg.contains("connection reset")
                {
                    tracing::debug!("HTTP/1 connection error from {}: {}", remote_addr, e);
                }
            }
        });
    }
}

// ── HTTP redirect server ───────────────────────────────────────────────

async fn run_http_redirect(port: u16, _base_domain: &str) -> anyhow::Result<()> {
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;

    let addr: SocketAddr = format!("[::]:{}", port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;

    info!("HTTP redirect listening on {}", addr);

    loop {
        let (stream, _remote) = match listener.accept().await {
            Ok(r) => r,
            Err(e) => {
                warn!("HTTP accept error: {}", e);
                continue;
            }
        };

        let io = TokioIo::new(stream);

        tokio::spawn(async move {
            let service = service_fn(|req: hyper::Request<hyper::body::Incoming>| async move {
                let host = req
                    .headers()
                    .get("host")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("localhost");
                let path = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
                let location = format!("https://{}{}", host, path);

                Ok::<_, std::convert::Infallible>(
                    hyper::Response::builder()
                        .status(301)
                        .header("Location", &location)
                        .body(http_body_util::Empty::<hyper::body::Bytes>::new())
                        .unwrap(),
                )
            });

            if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                let msg = e.to_string();
                if !msg.contains("connection closed") && !msg.contains("not connected") {
                    tracing::debug!("HTTP redirect error: {}", e);
                }
            }
        });
    }
}

// ── SIGHUP handler ─────────────────────────────────────────────────────

async fn handle_sighup(
    dns_dhcp_config_path: PathBuf,
    proxy_config_path: PathBuf,
    dns_state: hr_dns::SharedDnsState,
    proxy_state: Arc<ProxyState>,
    adblock: Arc<RwLock<AdblockEngine>>,
    tls_manager: Arc<TlsManager>,
) -> anyhow::Result<()> {
    let mut signals = Signals::new([SIGHUP])?;

    while let Some(signal) = signals.next().await {
        if signal == SIGHUP {
            info!("Received SIGHUP, reloading config...");

            // Reload DNS/DHCP config
            match DnsDhcpConfig::load(&dns_dhcp_config_path) {
                Ok(new_config) => {
                    let mut s = dns_state.write().await;
                    s.upstream = hr_dns::upstream::UpstreamForwarder::new(
                        new_config.dns.upstream_servers.clone(),
                        new_config.dns.upstream_timeout_ms,
                    );
                    s.config = new_config.dns;
                    s.adblock_enabled = new_config.adblock.enabled;
                    s.adblock_block_response = new_config.adblock.block_response;
                    s.dns_cache.clear().await;

                    let mut ab = adblock.write().await;
                    ab.set_whitelist(new_config.adblock.whitelist);

                    info!("DNS/DHCP config reloaded");
                }
                Err(e) => {
                    error!("Failed to reload DNS/DHCP config: {}", e);
                }
            }

            // Reload proxy config
            match ProxyConfig::load_from_file(&proxy_config_path) {
                Ok(new_config) => {
                    if let Err(e) = tls_manager.reload_certificates(&new_config.routes) {
                        error!("Failed to reload TLS certificates: {}", e);
                    }
                    proxy_state.reload_config(new_config);
                    info!("Proxy config reloaded");
                }
                Err(e) => {
                    error!("Failed to reload proxy config: {}", e);
                }
            }
        }
    }

    Ok(())
}

// ── Adblock update ─────────────────────────────────────────────────────

async fn do_adblock_update(
    adblock: &Arc<RwLock<AdblockEngine>>,
    sources: &[hr_adblock::config::AdblockSource],
    data_dir: &str,
    _dns_state: &hr_dns::SharedDnsState,
) {
    let (domains, _results) = hr_adblock::sources::download_all(sources).await;
    let count = domains.len();

    {
        let mut ab = adblock.write().await;
        ab.set_blocked(domains.clone());
    }

    let cache_path = PathBuf::from(data_dir).join("domains.json");
    if let Err(e) = hr_adblock::sources::save_cache(&domains, &cache_path) {
        warn!("Failed to save adblock cache: {}", e);
    }

    info!("Adblock update complete: {} unique domains blocked", count);
}
