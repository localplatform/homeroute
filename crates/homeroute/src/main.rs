mod supervisor;

use hr_adblock::AdblockEngine;
use hr_auth::AuthService;
use hr_acme::{AcmeConfig, AcmeManager, WildcardType};
use hr_common::config::EnvConfig;
use hr_common::events::{CertReadyEvent, EventBus};
use hr_common::service_registry::{
    new_service_registry, now_millis, ServicePriorityLevel, ServiceState, ServiceStatus,
};
use hr_dns::DnsState;
use hr_proxy::{ProxyConfig, ProxyState, TlsManager};
use hr_registry::AgentRegistry;
use signal_hook::consts::SIGHUP;
use signal_hook_tokio::Signals;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use supervisor::{spawn_supervised, ServicePriority};
use tokio::sync::RwLock;
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};

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

    // Initialize service registry
    let service_registry = new_service_registry();

    // Initialize auth service
    let auth = AuthService::new(&env.auth_data_dir, &env.base_domain)?;
    auth.start_cleanup_task();
    info!("Auth service initialized");

    // Initialize ACME (Let's Encrypt)
    let acme_config = AcmeConfig {
        storage_path: env.acme_storage_path.to_string_lossy().to_string(),
        cf_api_token: env.cf_api_token.clone().unwrap_or_default(),
        cf_zone_id: env.cf_zone_id.clone().unwrap_or_default(),
        base_domain: env.base_domain.clone(),
        directory_url: if env.acme_staging {
            "https://acme-staging-v02.api.letsencrypt.org/directory".to_string()
        } else {
            "https://acme-v02.api.letsencrypt.org/directory".to_string()
        },
        account_email: env.acme_email.clone()
            .unwrap_or_else(|| format!("admin@{}", env.base_domain)),
        renewal_threshold_days: 30,
    };
    let acme = Arc::new(AcmeManager::new(acme_config));
    acme.init().await?;
    info!(
        "ACME manager initialized ({})",
        if acme.is_initialized() {
            "account loaded"
        } else {
            "new account created"
        }
    );

    // Request global wildcard certificate if not present
    if acme.get_certificate(WildcardType::Global).is_err() {
        info!("Requesting global wildcard certificate...");
        match acme.request_wildcard(WildcardType::Global).await {
            Ok(cert) => info!("Global wildcard certificate issued: {} (expires {})", cert.id, cert.expires_at),
            Err(e) => warn!("Failed to request global wildcard: {}", e),
        }
    }

    // Remove legacy code wildcard certificate (replaced by per-app wildcards)
    if acme.get_certificate(WildcardType::LegacyCode).is_ok() {
        info!("Removing legacy code wildcard certificate (replaced by per-app wildcards)");
        let mut index = acme.list_certificates().unwrap_or_default();
        index.retain(|c| c.wildcard_type != WildcardType::LegacyCode);
        let _ = acme.storage().save_index(&index);
        let _ = std::fs::remove_file(acme.storage().cert_path(&WildcardType::LegacyCode));
        let _ = std::fs::remove_file(acme.storage().key_path(&WildcardType::LegacyCode));
        let _ = std::fs::remove_file(acme.storage().chain_path(&WildcardType::LegacyCode));
        info!("Legacy code wildcard certificate removed");
    }

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
    }));

    // ── Initialize proxy ───────────────────────────────────────────────

    let proxy_config_path = env.proxy_config_path.clone();
    let proxy_config = if proxy_config_path.exists() {
        ProxyConfig::load_from_file(&proxy_config_path)?
    } else {
        ProxyConfig {
            base_domain: env.base_domain.clone(),
            ca_storage_path: env.acme_storage_path.clone(),
            ..serde_json::from_str("{}")?
        }
    };

    let tls_manager = TlsManager::new(env.acme_storage_path.clone());

    // Load all certificates from ACME index (global, legacy code, and per-app wildcards)
    let certs = acme.list_certificates().unwrap_or_default();
    for cert_info in &certs {
        let cert_path = std::path::Path::new(&cert_info.cert_path);
        let key_path = std::path::Path::new(&cert_info.key_path);
        if cert_path.exists() && key_path.exists() {
            match tls_manager.load_cert_from_files(cert_path, key_path) {
                Ok(certified_key) => {
                    let domain = cert_info.wildcard_type.domain_pattern(&env.base_domain);
                    tls_manager.add_cert(&domain, certified_key);
                    info!(domain = %domain, "Loaded certificate");
                }
                Err(e) => {
                    warn!(cert_id = %cert_info.id, error = %e, "Failed to load certificate");
                }
            }
        }
    }

    // Set global wildcard as fallback for unknown SNI domains
    if let Ok(cert_info) = acme.get_certificate(WildcardType::Global) {
        if let Err(e) = tls_manager.set_fallback_certificate_from_pem(
            &cert_info.cert_path,
            &cert_info.key_path,
        ) {
            warn!("Failed to set fallback certificate: {}", e);
        }
    }

    let tls_config = tls_manager.build_server_config()?;

    let proxy_state = Arc::new(
        ProxyState::new(proxy_config.clone(), env.api_port)
            .with_auth(auth.clone()),
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
        // IPv6 addresses need brackets: [addr]:port
        let addr_formatted = if addr_str.contains(':') {
            format!("[{}]:{}", addr_str, dns_dhcp_config.dns.port)
        } else {
            format!("{}:{}", addr_str, dns_dhcp_config.dns.port)
        };
        let addr: SocketAddr = addr_formatted.parse()?;

        let dns_state_c = dns_state.clone();
        let reg = service_registry.clone();
        spawn_supervised("dns-udp", ServicePriority::Critical, reg, move || {
            let state = dns_state_c.clone();
            let addr = addr;
            async move { hr_dns::server::run_udp_server(addr, state).await }
        });

        let dns_state_c = dns_state.clone();
        let reg = service_registry.clone();
        spawn_supervised("dns-tcp", ServicePriority::Critical, reg, move || {
            let state = dns_state_c.clone();
            let addr = addr;
            async move { hr_dns::server::run_tcp_server(addr, state).await }
        });
    }

    // DHCP server (Critical)
    if dns_dhcp_config.dhcp.enabled {
        let dhcp_state_c = dhcp_state.clone();
        let reg = service_registry.clone();
        spawn_supervised("dhcp", ServicePriority::Critical, reg, move || {
            let state = dhcp_state_c.clone();
            async move { hr_dhcp::server::run_dhcp_server(state).await }
        });
    } else {
        let mut reg = service_registry.write().await;
        reg.insert("dhcp".into(), ServiceStatus {
            name: "dhcp".into(),
            state: ServiceState::Disabled,
            priority: ServicePriorityLevel::Critical,
            restart_count: 0,
            last_state_change: now_millis(),
            error: None,
        });
        drop(reg);
    }

    // HTTPS proxy (Critical)
    {
        let proxy_state_c = proxy_state.clone();
        let tls_config_c = tls_config.clone();
        let reg = service_registry.clone();
        spawn_supervised("proxy-https", ServicePriority::Critical, reg, move || {
            let proxy_state = proxy_state_c.clone();
            let tls_config = tls_config_c.clone();
            let port = https_port;
            async move { run_https_server(proxy_state, tls_config, port).await }
        });
    }

    // HTTP redirect (Critical)
    {
        let base_domain = env.base_domain.clone();
        let reg = service_registry.clone();
        spawn_supervised("proxy-http", ServicePriority::Critical, reg, move || {
            let base_domain = base_domain.clone();
            let port = http_port;
            async move { run_http_redirect(port, &base_domain).await }
        });
    }

    // Cloud Relay command channel (API → tunnel client for binary updates)
    let (cloud_relay_cmd_tx, cloud_relay_cmd_rx) =
        tokio::sync::mpsc::channel::<hr_common::events::CloudRelayCommand>(4);
    let cloud_relay_cmd_rx = Arc::new(tokio::sync::Mutex::new(cloud_relay_cmd_rx));

    // Cloud Relay enabled watch channel (API toggles, tunnel client reacts)
    let (cloud_relay_enabled_tx, cloud_relay_enabled_rx) =
        tokio::sync::watch::channel(env.cloud_relay_enabled);

    // Shared cloud relay status (written by tunnel client, read by API)
    let cloud_relay_status: Arc<tokio::sync::RwLock<Option<hr_api::state::CloudRelayInfo>>> =
        Arc::new(tokio::sync::RwLock::new(None));

    // Cloud Relay tunnel client — always spawned if host configured, waits for enable signal
    if let Some(ref relay_host) = env.cloud_relay_host {
        let relay_host = relay_host.clone();
        let relay_port = env.cloud_relay_quic_port;
        let data_dir = env.data_dir.clone();
        let proxy_state_c = proxy_state.clone();
        let tls_config_c = tls_config.clone();
        let events_c = events.clone();
        let cmd_rx = cloud_relay_cmd_rx.clone();
        let enabled_rx = cloud_relay_enabled_rx.clone();
        let status_handle = cloud_relay_status.clone();
        let reg = service_registry.clone();
        spawn_supervised(
            "cloud-relay-tunnel",
            ServicePriority::Critical,
            reg,
            move || {
                let relay_host = relay_host.clone();
                let data_dir = data_dir.clone();
                let proxy_state = proxy_state_c.clone();
                let tls_config = tls_config_c.clone();
                let events = events_c.clone();
                let cmd_rx = cmd_rx.clone();
                let enabled_rx = enabled_rx.clone();
                let status_handle = status_handle.clone();
                async move {
                    run_tunnel_client(
                        &relay_host,
                        relay_port,
                        &data_dir,
                        proxy_state,
                        tls_config,
                        events,
                        cmd_rx,
                        enabled_rx,
                        status_handle,
                    )
                    .await
                }
            },
        );
        info!(port = relay_port, "Cloud relay tunnel supervisor started");
    }

    // ── IPv6 Prefix Delegation + RA ─────────────────────────────────

    // Watch channel: PD client → RA sender
    let (prefix_tx, prefix_rx) =
        tokio::sync::watch::channel::<Option<hr_ipv6::PrefixInfo>>(None);

    // 1) DHCPv6-PD client (obtains /56 from upstream, publishes /64 on channel)
    if dns_dhcp_config.ipv6.enabled && dns_dhcp_config.ipv6.pd_enabled {
        let ipv6_config = dns_dhcp_config.ipv6.clone();
        let tx = prefix_tx.clone();
        let reg = service_registry.clone();
        spawn_supervised("ipv6-pd", ServicePriority::Important, reg, move || {
            let config = ipv6_config.clone();
            let tx = tx.clone();
            async move { hr_ipv6::pd_client::run_pd_client(config, tx).await }
        });
    } else {
        let mut reg = service_registry.write().await;
        reg.insert("ipv6-pd".into(), ServiceStatus {
            name: "ipv6-pd".into(),
            state: ServiceState::Disabled,
            priority: ServicePriorityLevel::Important,
            restart_count: 0,
            last_state_change: now_millis(),
            error: None,
        });
        drop(reg);
    }

    // 2) IPv6 RA sender (announces ULA + GUA prefixes)
    if dns_dhcp_config.ipv6.enabled && dns_dhcp_config.ipv6.ra_enabled {
        let ipv6_config = dns_dhcp_config.ipv6.clone();
        let rx = prefix_rx.clone();
        let reg = service_registry.clone();
        spawn_supervised("ipv6-ra", ServicePriority::Important, reg, move || {
            let config = ipv6_config.clone();
            let rx = rx.clone();
            async move { hr_ipv6::ra::run_ra_sender(config, rx).await }
        });
    } else {
        let mut reg = service_registry.write().await;
        reg.insert("ipv6-ra".into(), ServiceStatus {
            name: "ipv6-ra".into(),
            state: ServiceState::Disabled,
            priority: ServicePriorityLevel::Important,
            restart_count: 0,
            last_state_change: now_millis(),
            error: None,
        });
        drop(reg);
    }

    // 3) DHCPv6 stateful server (assigns addresses from GUA prefix)
    if dns_dhcp_config.ipv6.enabled && dns_dhcp_config.ipv6.dhcpv6_enabled {
        let ipv6_config = dns_dhcp_config.ipv6.clone();
        let rx = prefix_rx.clone();
        let reg = service_registry.clone();
        spawn_supervised("dhcpv6", ServicePriority::Important, reg, move || {
            let config = ipv6_config.clone();
            let prefix_rx = rx.clone();
            async move { hr_ipv6::dhcpv6::run_dhcpv6_server(config, prefix_rx).await }
        });
    } else {
        let mut reg = service_registry.write().await;
        reg.insert("dhcpv6".into(), ServiceStatus {
            name: "dhcpv6".into(),
            state: ServiceState::Disabled,
            priority: ServicePriorityLevel::Important,
            restart_count: 0,
            last_state_change: now_millis(),
            error: None,
        });
        drop(reg);
    }

    // ── Agent Registry ──────────────────────────────────────────────

    let registry_state_path =
        PathBuf::from("/var/lib/server-dashboard/agent-registry.json");
    let registry = Arc::new(AgentRegistry::new(
        registry_state_path,
        Arc::new(env.clone()),
        events.clone(),
    ));

    // Provide ACME manager to registry for per-app certificate management
    registry.set_acme(acme.clone()).await;

    // Request per-app wildcard certificates for existing applications that don't have one yet
    {
        let apps = registry.list_applications().await;
        let acme_init = acme.clone();
        let events_init = events.clone();
        let base_domain_init = env.base_domain.clone();
        let tls_init = tls_manager.clone();
        let missing_apps: Vec<_> = apps
            .iter()
            .filter(|app| acme_init.get_app_certificate(&app.slug).is_err())
            .map(|app| app.slug.clone())
            .collect();

        if !missing_apps.is_empty() {
            info!(
                count = missing_apps.len(),
                "Requesting per-app wildcard certificates for existing applications"
            );
            tokio::spawn(async move {
                for slug in missing_apps {
                    info!(slug = %slug, "Requesting per-app wildcard certificate");
                    match acme_init.request_app_wildcard(&slug).await {
                        Ok(cert) => {
                            let domain = cert.wildcard_type.domain_pattern(&base_domain_init);
                            // Load into TLS manager immediately
                            let cert_path = std::path::Path::new(&cert.cert_path);
                            let key_path = std::path::Path::new(&cert.key_path);
                            if let Ok(certified_key) = tls_init.load_cert_from_files(cert_path, key_path) {
                                tls_init.add_cert(&domain, certified_key);
                            }
                            let _ = events_init.cert_ready.send(CertReadyEvent {
                                slug: slug.clone(),
                                wildcard_domain: domain.clone(),
                                cert_path: cert.cert_path.clone(),
                                key_path: cert.key_path.clone(),
                            });
                            info!(slug = %slug, domain = %domain, "Per-app wildcard certificate issued");
                        }
                        Err(e) => {
                            warn!(slug = %slug, error = %e, "Failed to request per-app wildcard certificate");
                        }
                    }
                    // Stagger requests to avoid Let's Encrypt rate limits
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
            });
        }
    }

    // Heartbeat monitor
    {
        let reg = registry.clone();
        tokio::spawn(async move {
            reg.run_heartbeat_monitor().await;
        });
    }

    // Connect proxy to registry for ActivityPing and Wake-on-Demand
    proxy_state.set_registry(registry.clone());
    proxy_state.set_events(events.clone());

    // Populate app routes for all applications with IPv4 addresses
    {
        let apps = registry.list_applications().await;
        let base_domain = &env.base_domain;
        for app in &apps {
            if let Some(ipv4) = app.ipv4_address {
                for route in app.routes(base_domain) {
                    proxy_state.set_app_route(
                        route.domain,
                        hr_proxy::AppRoute {
                            app_id: app.id.clone(),
                            host_id: app.host_id.clone(),
                            target_ip: ipv4,
                            target_port: route.target_port,
                            auth_required: route.auth_required,
                            allowed_groups: route.allowed_groups,
                            service_type: route.service_type,
                            wake_page_enabled: app.wake_page_enabled,
                            local_only: app.frontend.local_only,
                        },
                    );
                }
            }
        }
    }

    info!(
        "Agent registry initialized ({} applications)",
        registry.list_applications().await.len()
    );

    // ── Container V2 Manager (nspawn) ────────────────────────────────

    let container_v2_state_path = PathBuf::from("/var/lib/server-dashboard/containers-v2.json");
    let container_manager = Arc::new(hr_api::container_manager::ContainerManager::new(
        container_v2_state_path,
        Arc::new(env.clone()),
        events.clone(),
        registry.clone(),
    ));

    // ── Restore local containers that were running before reboot ──────
    container_manager.restore_local_containers().await;

    // ── Management API (Important) ────────────────────────────────────

    let api_state = hr_api::state::ApiState {
        auth: auth.clone(),
        acme: acme.clone(),
        proxy: proxy_state.clone(),
        tls_manager: tls_manager.clone(),
        dns: dns_state.clone(),
        dhcp: dhcp_state.clone(),
        adblock: adblock.clone(),
        events: events.clone(),
        env: Arc::new(env.clone()),
        dns_dhcp_config_path: env.dns_dhcp_config_path.clone(),
        proxy_config_path: env.proxy_config_path.clone(),
        reverseproxy_config_path: env.reverseproxy_config_path.clone(),
        service_registry: service_registry.clone(),

        registry: Some(registry.clone()),
        container_manager: Some(container_manager.clone()),
        migrations: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        renames: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        dataverse_schemas: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        cloud_relay_status: cloud_relay_status.clone(),
        cloud_relay_enabled: cloud_relay_enabled_tx,
        cloud_relay_cmd_tx: Some(cloud_relay_cmd_tx),
    };

    let api_router = hr_api::build_router(api_state);
    let api_port = env.api_port;

    let reg = service_registry.clone();
    spawn_supervised("api", ServicePriority::Important, reg, move || {
        let router = api_router.clone();
        let port = api_port;
        async move {
            let addr: SocketAddr = format!("[::]:{}", port).parse()?;
            let listener = tokio::net::TcpListener::bind(addr).await?;
            info!("Management API listening on {}", addr);
            axum::serve(listener, router).await?;
            Ok(())
        }
    });

    // ── Background tasks ───────────────────────────────────────────────

    // Lease persistence + expired lease purge (every 60s)
    {
        let dhcp_state_c = dhcp_state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                let mut s = dhcp_state_c.write().await;
                let purged = s.lease_store.purge_expired();
                if purged > 0 {
                    info!("Purged {} expired DHCP leases", purged);
                }
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

    // CertReady listener — dynamically load new certificates into TLS manager
    // and notify agents of certificate renewals
    {
        let tls_mgr = tls_manager.clone();
        let registry_cert = registry.clone();
        let mut cert_rx = events.cert_ready.subscribe();
        tokio::spawn(async move {
            while let Ok(event) = cert_rx.recv().await {
                let cert_path = std::path::Path::new(&event.cert_path);
                let key_path = std::path::Path::new(&event.key_path);
                match tls_mgr.load_cert_from_files(cert_path, key_path) {
                    Ok(certified_key) => {
                        tls_mgr.add_cert(&event.wildcard_domain, certified_key);
                        info!(domain = %event.wildcard_domain, "Dynamically loaded new certificate");
                    }
                    Err(e) => {
                        warn!(domain = %event.wildcard_domain, error = %e, "Failed to load dynamic certificate");
                    }
                }

                // Notify agents of certificate renewal so they can hot-reload
                if event.slug.is_empty() {
                    // Global cert — notify ALL connected agents
                    let apps = registry_cert.list_applications().await;
                    for app in &apps {
                        if registry_cert.is_agent_connected(&app.id).await {
                            if let Err(e) = registry_cert
                                .send_to_agent(
                                    &app.id,
                                    hr_registry::RegistryMessage::CertRenewal {
                                        slug: String::new(),
                                    },
                                )
                                .await
                            {
                                warn!(app = %app.slug, error = %e, "Failed to send CertRenewal to agent");
                            }
                        }
                    }
                    info!("Sent global CertRenewal notification to all connected agents");
                } else {
                    // Per-app cert — notify only the matching agent
                    let apps = registry_cert.list_applications().await;
                    if let Some(app) = apps.iter().find(|a| a.slug == event.slug) {
                        if registry_cert.is_agent_connected(&app.id).await {
                            if let Err(e) = registry_cert
                                .send_to_agent(
                                    &app.id,
                                    hr_registry::RegistryMessage::CertRenewal {
                                        slug: event.slug.clone(),
                                    },
                                )
                                .await
                            {
                                warn!(slug = %event.slug, error = %e, "Failed to send CertRenewal to agent");
                            } else {
                                info!(slug = %event.slug, "Sent CertRenewal notification to agent");
                            }
                        }
                    }
                }
            }
        });
    }

    // Certificate renewal task — checks every 12 hours for expiring certificates
    {
        let acme_renewal = acme.clone();
        let events_renewal = events.clone();
        let base_domain_renewal = env.base_domain.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(12 * 3600)).await;
                info!("Checking for certificate renewals...");
                match acme_renewal.certificates_needing_renewal() {
                    Ok(certs) if !certs.is_empty() => {
                        for cert_info in certs {
                            info!(cert_id = %cert_info.id, "Renewing certificate");
                            match acme_renewal.request_wildcard(cert_info.wildcard_type.clone()).await {
                                Ok(new_cert) => {
                                    let domain = new_cert.wildcard_type.domain_pattern(&base_domain_renewal);
                                    let _ = events_renewal.cert_ready.send(CertReadyEvent {
                                        slug: match &new_cert.wildcard_type {
                                            hr_acme::WildcardType::App { slug } => slug.clone(),
                                            _ => String::new(),
                                        },
                                        wildcard_domain: domain,
                                        cert_path: new_cert.cert_path.clone(),
                                        key_path: new_cert.key_path.clone(),
                                    });
                                    info!(cert_id = %new_cert.id, "Certificate renewed successfully");
                                }
                                Err(e) => {
                                    warn!(cert_id = %cert_info.id, error = %e, "Failed to renew certificate");
                                }
                            }
                            // Stagger renewals to avoid rate limits
                            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                        }
                    }
                    Ok(_) => debug!("No certificates need renewal"),
                    Err(e) => warn!(error = %e, "Failed to check certificate renewals"),
                }
            }
        });
    }

    // Migrate servers.json → hosts.json if needed
    hr_api::routes::hosts::ensure_hosts_file().await;

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

    // ── Startup cleanup + periodic maintenance ─────────────────────────

    // Clean up stale migration transfer files from /tmp
    tokio::spawn(async {
        if let Ok(mut entries) = tokio::fs::read_dir("/tmp").await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.ends_with(".tar.gz") && name_str.len() > 40 {
                    let stem = &name_str[..name_str.len() - 7];
                    if uuid::Uuid::parse_str(stem).is_ok() {
                        tracing::info!("Cleaning stale migration file: /tmp/{}", name_str);
                        let _ = tokio::fs::remove_file(entry.path()).await;
                    }
                }
            }
        }
    });

    // Periodic cleanup of stale migration/exec signals
    {
        let reg = registry.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                reg.cleanup_stale_signals().await;
            }
        });
    }

    // ── Ready ──────────────────────────────────────────────────────────

    info!("HomeRoute started successfully");
    info!("  Auth: OK");
    info!(
        "  ACME: OK ({} wildcard certificates)",
        acme.list_certificates().unwrap_or_default().len()
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
    info!("  Hosts: status via host-agent WebSocket");

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

// ── Cloud Relay tunnel client ─────────────────────────────────────────

async fn run_tunnel_client(
    relay_host: &str,
    relay_port: u16,
    data_dir: &std::path::Path,
    proxy_state: Arc<ProxyState>,
    tls_config: Arc<rustls::ServerConfig>,
    events: Arc<EventBus>,
    cmd_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<hr_common::events::CloudRelayCommand>>>,
    mut enabled_rx: tokio::sync::watch::Receiver<bool>,
    status_handle: Arc<tokio::sync::RwLock<Option<hr_api::state::CloudRelayInfo>>>,
) -> anyhow::Result<()> {
    use hr_common::events::{CloudRelayCommand, CloudRelayEvent, CloudRelayStatus};
    use hr_tunnel::protocol::StreamHeader;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use tokio_rustls::TlsAcceptor;

    // Helper: update shared status for API
    let update_status = |status_handle: &Arc<tokio::sync::RwLock<Option<hr_api::state::CloudRelayInfo>>>,
                         status: CloudRelayStatus,
                         vps_ipv4: Option<String>| {
        let handle = status_handle.clone();
        async move {
            *handle.write().await = Some(hr_api::state::CloudRelayInfo {
                status,
                vps_ipv4,
                latency_ms: None,
                active_streams: None,
            });
        }
    };

    // ── Wait until relay is enabled ──────────────────────────────────
    loop {
        if *enabled_rx.borrow_and_update() {
            break;
        }
        info!("Cloud relay disabled, tunnel waiting for enable signal...");
        update_status(&status_handle, CloudRelayStatus::Disconnected, None).await;
        let _ = events.cloud_relay.send(CloudRelayEvent {
            status: CloudRelayStatus::Disconnected,
            latency_ms: None,
            active_streams: None,
            message: Some("Waiting for enable".to_string()),
        });
        enabled_rx
            .changed()
            .await
            .map_err(|_| anyhow::anyhow!("Enabled watch channel closed"))?;
    }

    // ── Connect to VPS ───────────────────────────────────────────────
    let relay_dir = data_dir.join("cloud-relay");

    // Load mTLS client certificates
    let ca_pem = tokio::fs::read(relay_dir.join("ca.pem")).await?;
    let client_pem = tokio::fs::read(relay_dir.join("client.pem")).await?;
    let client_key_pem = tokio::fs::read(relay_dir.join("client-key.pem")).await?;

    let client_config =
        hr_tunnel::quic::build_client_config(&client_pem, &client_key_pem, &ca_pem)?;

    // Create QUIC endpoint (bind ephemeral port)
    let mut endpoint = quinn::Endpoint::client("[::]:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    // Resolve hostname to IP (SocketAddr::parse only accepts IPs, not hostnames)
    let server_addr = tokio::net::lookup_host(format!("{}:{}", relay_host, relay_port))
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("Failed to resolve relay host: {}", relay_host))?;
    let server_name = relay_host.to_string();

    info!(host = %relay_host, port = relay_port, "Connecting QUIC tunnel to cloud relay...");

    let _ = events.cloud_relay.send(CloudRelayEvent {
        status: CloudRelayStatus::Reconnecting,
        latency_ms: None,
        active_streams: None,
        message: Some(format!("Connecting to {}:{}", relay_host, relay_port)),
    });

    // Connect to VPS
    let connection = endpoint.connect(server_addr, &server_name)?.await?;

    info!("QUIC tunnel connected to {}", connection.remote_address());

    // Read VPS IPv4 from config for status
    let vps_ipv4 = load_relay_vps_ipv4(data_dir);
    update_status(&status_handle, CloudRelayStatus::Connected, vps_ipv4).await;
    let _ = events.cloud_relay.send(CloudRelayEvent {
        status: CloudRelayStatus::Connected,
        latency_ms: None,
        active_streams: None,
        message: Some("Tunnel connected".to_string()),
    });

    let tls_acceptor = TlsAcceptor::from(tls_config);

    // Lock the command receiver for this tunnel session
    let mut cmd_rx = cmd_rx.lock().await;

    // Accept incoming bidirectional streams (each = one TCP connection from the internet)
    loop {
        let (mut quic_send, mut quic_recv) = tokio::select! {
            result = connection.accept_bi() => {
                match result {
                    Ok(streams) => streams,
                    Err(e) => {
                        warn!("QUIC tunnel closed: {}", e);
                        update_status(&status_handle, CloudRelayStatus::Disconnected, None).await;
                        let _ = events.cloud_relay.send(CloudRelayEvent {
                            status: CloudRelayStatus::Disconnected,
                            latency_ms: None,
                            active_streams: None,
                            message: Some(format!("Tunnel closed: {}", e)),
                        });
                        return Err(e.into());
                    }
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(CloudRelayCommand::PushBinaryUpdate { binary_data, sha256, response_tx }) => {
                        let result = push_binary_update(&connection, &binary_data, &sha256).await;
                        let _ = response_tx.send(result);
                    }
                    None => {
                        // Channel closed, continue accepting streams
                    }
                }
                continue;
            }
            _ = enabled_rx.changed() => {
                if !*enabled_rx.borrow() {
                    info!("Cloud relay disabled by user, closing tunnel");
                    connection.close(0u32.into(), b"disabled");
                    update_status(&status_handle, CloudRelayStatus::Disconnected, None).await;
                    let _ = events.cloud_relay.send(CloudRelayEvent {
                        status: CloudRelayStatus::Disconnected,
                        latency_ms: None,
                        active_streams: None,
                        message: Some("Tunnel disabled by user".to_string()),
                    });
                    // Return error so supervisor restarts — will block at wait-for-enable
                    anyhow::bail!("Cloud relay disabled");
                }
                continue;
            }
        };

        let proxy_state = proxy_state.clone();
        let acceptor = tls_acceptor.clone();

        tokio::spawn(async move {
            // Read the StreamHeader to get client IP
            let mut header_buf = vec![0u8; 26]; // max: 1 + 1 + 16 + 8 = 26
            let n = match quic_recv.read(&mut header_buf).await {
                Ok(Some(n)) => n,
                Ok(None) => return,
                Err(e) => {
                    tracing::debug!("Failed to read stream header: {}", e);
                    return;
                }
            };

            let mut cursor = &header_buf[..n];
            let header = match StreamHeader::decode(&mut cursor) {
                Ok(h) => h,
                Err(e) => {
                    tracing::debug!("Invalid stream header: {}", e);
                    return;
                }
            };

            let client_ip = header.client_ip;

            // Bridge QUIC streams to a single AsyncRead+AsyncWrite via duplex
            let (quic_side, tls_side) = tokio::io::duplex(256 * 1024);
            let (quic_reader, mut quic_writer) = tokio::io::split(quic_side);

            // Task: QUIC recv → quic_writer → tls_side (readable by TLS acceptor)
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let mut buf = vec![0u8; 65536];
                loop {
                    match quic_recv.read(&mut buf).await {
                        Ok(Some(n)) => {
                            if quic_writer.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
            });

            // Task: quic_reader (data written by TLS) → QUIC send
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut reader = quic_reader;
                let mut buf = vec![0u8; 65536];
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            if quic_send.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            });

            // TLS termination on the duplex stream (Cloudflare → on-prem handshake)
            let tls_stream = match acceptor.accept(tls_side).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!(
                        "TLS handshake failed from relay (client {}): {}",
                        client_ip,
                        e
                    );
                    return;
                }
            };

            let io = TokioIo::new(tls_stream);
            let service = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                let state = proxy_state.clone();
                async move {
                    let (parts, body) = req.into_parts();
                    let req =
                        axum::extract::Request::from_parts(parts, axum::body::Body::new(body));
                    let resp = hr_proxy::proxy_handler(state, client_ip, req).await;
                    Ok::<_, std::convert::Infallible>(axum::response::IntoResponse::into_response(
                        resp,
                    ))
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
                    tracing::debug!(
                        "HTTP/1 relay connection error (client {}): {}",
                        client_ip,
                        e
                    );
                }
            }
        });
    }
}

/// Push a binary update to the VPS via a QUIC unidirectional stream.
/// Format: [4-byte length][JSON ControlMessage::BinaryUpdate][raw binary bytes]
async fn push_binary_update(
    connection: &quinn::Connection,
    binary_data: &[u8],
    sha256: &str,
) -> Result<String, String> {
    use hr_tunnel::protocol::ControlMessage;

    // Open a uni stream to the VPS
    let mut send = connection
        .open_uni()
        .await
        .map_err(|e| format!("Failed to open QUIC stream: {}", e))?;

    // Send the control message header
    let msg = ControlMessage::BinaryUpdate {
        size: binary_data.len() as u64,
        sha256: sha256.to_string(),
    };
    let encoded = msg
        .encode()
        .map_err(|e| format!("Failed to encode message: {}", e))?;
    send.write_all(&encoded)
        .await
        .map_err(|e| format!("Failed to send header: {}", e))?;

    // Send the raw binary data
    send.write_all(binary_data)
        .await
        .map_err(|e| format!("Failed to send binary: {}", e))?;

    send.finish()
        .map_err(|e| format!("Failed to finish stream: {}", e))?;

    Ok(format!(
        "Binary ({} bytes) pushed to VPS, service restarting",
        binary_data.len()
    ))
}

/// Read VPS IPv4 from relay config.json (best-effort).
fn load_relay_vps_ipv4(data_dir: &std::path::Path) -> Option<String> {
    let path = data_dir.join("cloud-relay/config.json");
    let content = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("vps_ipv4")?.as_str().map(|s| s.to_string())
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
            // TLS termination — all routing is handled at HTTP level by hr-proxy
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
