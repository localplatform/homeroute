mod config;
mod connection;
mod ipv6;
mod metrics;
mod pages;
mod powersave;
mod proxy;
mod services;
mod update;

use std::net::Ipv6Addr;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use hr_registry::protocol::{AgentMessage, AgentMetrics, RegistryMessage, ServiceConfig, ServiceState};

use crate::metrics::MetricsCollector;
use crate::powersave::{PowersaveManager, ServiceStateChange};
use crate::services::ServiceManager;

const CONFIG_PATH: &str = "/etc/hr-agent.toml";
const MAX_BACKOFF_SECS: u64 = 60;
const INITIAL_BACKOFF_SECS: u64 = 5;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,hr_agent=debug".parse().unwrap()),
        )
        .init();

    // Install rustls crypto provider
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    info!("HomeRoute Agent starting...");

    let cfg = config::AgentConfig::load(CONFIG_PATH)?;
    info!(
        service = cfg.service_name,
        homeroute = format!("{}:{}", cfg.homeroute_address, cfg.homeroute_port),
        "Config loaded"
    );

    // Create the proxy (not yet listening — needs routes from HomeRoute)
    let mut agent_proxy = proxy::AgentProxy::new()?;

    // Create metrics collector
    let metrics_collector = Arc::new(MetricsCollector::new());

    // Create service manager with empty config (will be updated from registry)
    let service_manager = Arc::new(RwLock::new(ServiceManager::new(&ServiceConfig::default())));

    // Create powersave manager
    let powersave_manager = Arc::new(PowersaveManager::new(Arc::clone(&service_manager)));

    // Connect powersave manager to the proxy for wake-on-request
    agent_proxy.state().set_powersave(Arc::clone(&powersave_manager));

    // Set app ID for WebSocket metrics filtering
    agent_proxy.state().set_app_id(cfg.service_name.clone());

    // Current assigned IPv6 address (if any)
    let mut current_ipv6: Option<String> = None;
    // Proxy task handle
    let mut proxy_handle: Option<tokio::task::JoinHandle<()>> = None;

    // Reconnection loop with exponential backoff
    let mut backoff = INITIAL_BACKOFF_SECS;

    loop {
        let (registry_tx, mut registry_rx) = mpsc::channel::<RegistryMessage>(32);
        let (outbound_tx, outbound_rx) = mpsc::channel::<AgentMessage>(64);

        // Channel for service state changes (powersave -> main for potential logging)
        let (state_change_tx, mut state_change_rx) = mpsc::channel::<ServiceStateChange>(16);

        info!(backoff_secs = backoff, "Connecting to HomeRoute...");

        // Spawn the WebSocket connection in a task so we can process messages concurrently
        let cfg_clone = cfg.clone();
        let mut conn_handle = tokio::spawn(async move {
            connection::run_connection(&cfg_clone, registry_tx, outbound_rx).await
        });

        // Spawn metrics sender task (1 second interval)
        let metrics_tx = outbound_tx.clone();
        let metrics_coll = Arc::clone(&metrics_collector);
        let powersave_mgr = Arc::clone(&powersave_manager);
        let metrics_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;

                // Collect metrics
                let memory_bytes = metrics_coll.memory_bytes().await;
                let cpu_percent = metrics_coll.cpu_percent().await;

                // Get service states from powersave manager
                let code_server_status = powersave_mgr.get_state(hr_registry::protocol::ServiceType::CodeServer);
                let app_status = powersave_mgr.get_state(hr_registry::protocol::ServiceType::App);
                let db_status = powersave_mgr.get_state(hr_registry::protocol::ServiceType::Db);

                // Get idle times
                let code_server_idle_secs = powersave_mgr.idle_secs(hr_registry::protocol::ServiceType::CodeServer);
                let app_idle_secs = powersave_mgr.idle_secs(hr_registry::protocol::ServiceType::App);

                let metrics = AgentMetrics {
                    code_server_status,
                    app_status,
                    db_status,
                    memory_bytes,
                    cpu_percent,
                    code_server_idle_secs,
                    app_idle_secs,
                };

                if metrics_tx.send(AgentMessage::Metrics(metrics)).await.is_err() {
                    // Channel closed, connection ended
                    break;
                }
            }
        });

        // Spawn idle checker task
        let powersave_for_idle = Arc::clone(&powersave_manager);
        let state_tx_for_idle = state_change_tx.clone();
        let idle_checker_handle = tokio::spawn(async move {
            powersave_for_idle.run_idle_checker(state_tx_for_idle).await;
        });

        // Process messages while the connection is alive
        let mut connected = false;
        loop {
            tokio::select! {
                // Connection task finished
                result = &mut conn_handle => {
                    match result {
                        Ok(Ok(())) => {
                            if connected {
                                info!("Connection closed cleanly, will reconnect");
                            }
                        }
                        Ok(Err(e)) => {
                            if connected {
                                error!("Connection lost: {e}");
                            } else {
                                error!("Connection failed: {e}");
                            }
                        }
                        Err(e) => {
                            error!("Connection task panicked: {e}");
                        }
                    }
                    break;
                }
                // Incoming message from the connection
                msg = registry_rx.recv() => {
                    match msg {
                        Some(msg) => {
                            if !connected {
                                connected = true;
                                backoff = INITIAL_BACKOFF_SECS;
                            }
                            handle_registry_message(
                                &cfg,
                                &mut agent_proxy,
                                &mut current_ipv6,
                                &mut proxy_handle,
                                &service_manager,
                                &powersave_manager,
                                &state_change_tx,
                                &outbound_tx,
                                msg
                            ).await;
                        }
                        None => {
                            // Channel closed — connection is done
                            break;
                        }
                    }
                }
                // Service state changes from powersave
                Some(change) = state_change_rx.recv() => {
                    info!(
                        service_type = ?change.service_type,
                        new_state = ?change.new_state,
                        "Service state changed"
                    );
                    // Could send ServiceStateChanged message here if needed
                }
            }
        }

        // Cancel background tasks
        metrics_handle.abort();
        idle_checker_handle.abort();

        // Drain any remaining messages
        while let Ok(msg) = registry_rx.try_recv() {
            handle_registry_message(
                &cfg,
                &mut agent_proxy,
                &mut current_ipv6,
                &mut proxy_handle,
                &service_manager,
                &powersave_manager,
                &state_change_tx,
                &outbound_tx,
                msg
            ).await;
        }

        // Wait before reconnecting
        info!(secs = backoff, "Waiting before reconnect...");
        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;

        // Exponential backoff (cap at MAX)
        backoff = (backoff * 2).min(MAX_BACKOFF_SECS);
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_registry_message(
    cfg: &config::AgentConfig,
    proxy: &mut proxy::AgentProxy,
    current_ipv6: &mut Option<String>,
    proxy_handle: &mut Option<tokio::task::JoinHandle<()>>,
    service_manager: &Arc<RwLock<ServiceManager>>,
    powersave_manager: &Arc<PowersaveManager>,
    state_change_tx: &mpsc::Sender<ServiceStateChange>,
    outbound_tx: &mpsc::Sender<AgentMessage>,
    msg: RegistryMessage,
) {
    match msg {
        RegistryMessage::Config {
            ipv6_address,
            routes,
            homeroute_auth_url,
            dashboard_url,
            services,
            power_policy,
            ..
        } => {
            info!(
                routes = routes.len(),
                ipv6 = ipv6_address,
                dashboard_url = dashboard_url,
                "Received full config from HomeRoute"
            );

            // Update service manager config
            {
                let mut mgr = service_manager.write().unwrap();
                mgr.update_config(&services);
            }

            // Update power policy
            powersave_manager.set_policy(&power_policy);

            // Apply IPv6 address
            if !ipv6_address.is_empty() {
                apply_ipv6(cfg, current_ipv6, &ipv6_address).await;
            }

            // Apply routes to proxy
            if let Err(e) = proxy.apply_routes(&routes, &homeroute_auth_url) {
                error!("Failed to apply routes: {e}");
                return;
            }

            // Set dashboard URL for loading/down pages
            if !dashboard_url.is_empty() {
                proxy.state().set_dashboard_url(dashboard_url);
            }

            // Start or restart the proxy if we have an IPv6 address
            if let Some(addr_str) = current_ipv6.as_deref() {
                // Wait briefly for the IPv6 address to pass DAD and become available
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                start_proxy(proxy, proxy_handle, addr_str).await;
            }
        }

        RegistryMessage::Ipv6Update { ipv6_address } => {
            info!(ipv6 = ipv6_address, "IPv6 address updated");
            apply_ipv6(cfg, current_ipv6, &ipv6_address).await;

            // Restart proxy on new address
            if let Some(addr_str) = current_ipv6.as_deref() {
                start_proxy(proxy, proxy_handle, addr_str).await;
            }
        }

        RegistryMessage::CertUpdate { routes } => {
            info!(routes = routes.len(), "Certificate update received");
            let auth_url = String::new(); // Keep existing auth_url
            if let Err(e) = proxy.apply_routes(&routes, &auth_url) {
                error!("Failed to apply cert update: {e}");
            }
        }

        RegistryMessage::Shutdown => {
            info!("Shutdown requested by HomeRoute");
            proxy.shutdown();
            if let Some(handle) = proxy_handle.take() {
                handle.abort();
            }
            std::process::exit(0);
        }

        RegistryMessage::AuthResult { .. } => {
            // Handled in connection.rs
        }

        RegistryMessage::UpdateAvailable { version, download_url, sha256 } => {
            info!(version, download_url, "Update available, starting auto-update");
            if let Err(e) = update::apply_update(&download_url, &sha256, &version).await {
                error!("Auto-update failed: {e}");
            }
        }

        RegistryMessage::PowerPolicyUpdate(policy) => {
            info!(
                code_server_timeout = ?policy.code_server_idle_timeout_secs,
                app_timeout = ?policy.app_idle_timeout_secs,
                "Power policy update received"
            );
            powersave_manager.set_policy(&policy);
        }

        RegistryMessage::ServiceCommand { service_type, action } => {
            info!(
                service_type = ?service_type,
                action = ?action,
                "Service command received"
            );
            powersave_manager.handle_command(service_type, action, state_change_tx).await;

            // Wait 1s for service to fully start/stop before notifying registry
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            // Send state change notification to registry
            let new_state = powersave_manager.get_state(service_type);
            let _ = outbound_tx
                .send(AgentMessage::ServiceStateChanged {
                    service_type,
                    new_state,
                })
                .await;
        }
    }
}

async fn apply_ipv6(
    cfg: &config::AgentConfig,
    current_ipv6: &mut Option<String>,
    new_addr: &str,
) {
    // Remove old address if different
    if let Some(old) = current_ipv6.as_ref() {
        if old != new_addr {
            if let Err(e) = ipv6::remove_address(&cfg.interface, old).await {
                warn!("Failed to remove old IPv6: {e}");
            }
        }
    }

    // Add new address
    if let Err(e) = ipv6::add_address(&cfg.interface, new_addr).await {
        error!("Failed to add IPv6 {new_addr}: {e}");
        return;
    }

    *current_ipv6 = Some(new_addr.to_string());
}

async fn start_proxy(
    proxy: &mut proxy::AgentProxy,
    proxy_handle: &mut Option<tokio::task::JoinHandle<()>>,
    addr_str: &str,
) {
    // Stop existing proxy
    proxy.shutdown();
    if let Some(handle) = proxy_handle.take() {
        handle.abort();
        // Give the old listener a moment to release the port
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let bind_addr: Ipv6Addr = match addr_str.parse() {
        Ok(a) => a,
        Err(e) => {
            error!("Invalid IPv6 address {addr_str}: {e}");
            return;
        }
    };

    info!(addr = addr_str, "Starting HTTPS proxy on [{}]:443", addr_str);

    // Get the shared components needed for the spawned task
    let listener_handle = proxy.spawn_listener(bind_addr);
    match listener_handle {
        Ok(handle) => {
            *proxy_handle = Some(handle);
        }
        Err(e) => {
            error!("Failed to start proxy listener: {e}");
        }
    }
}

/// Extract base URL from a full URL (e.g., "https://hr.example.com/api/..." -> "https://hr.example.com").
fn extract_base_url(url: &str) -> Option<String> {
    // Find scheme separator
    let scheme_end = url.find("://")?;
    let after_scheme = &url[scheme_end + 3..];

    // Find end of host (first slash after scheme, or end of string)
    let host_end = after_scheme.find('/').unwrap_or(after_scheme.len());
    let host = &after_scheme[..host_end];

    if host.is_empty() {
        return None;
    }

    Some(format!("{}://{}", &url[..scheme_end], host))
}
