mod config;
mod connection;
mod metrics;
mod powersave;
mod services;
mod update;

use std::sync::{Arc, RwLock};

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{error, info};

use hr_registry::protocol::{AgentMessage, AgentMetrics, AgentRoute, RegistryMessage, ServiceConfig, ServiceState, ServiceType};

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

    info!("HomeRoute Agent starting...");

    let cfg = config::AgentConfig::load(CONFIG_PATH)?;
    info!(
        service = cfg.service_name,
        homeroute = format!("{}:{}", cfg.homeroute_address, cfg.homeroute_port),
        "Config loaded"
    );

    // Create metrics collector
    let metrics_collector = Arc::new(MetricsCollector::new());

    // Create service manager with empty config (will be updated from registry)
    let service_manager = Arc::new(RwLock::new(ServiceManager::new(&ServiceConfig::default())));

    // Create powersave manager
    let powersave_manager = Arc::new(PowersaveManager::new(Arc::clone(&service_manager)));

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
                }
            }
        }

        // Cancel background tasks
        metrics_handle.abort();
        idle_checker_handle.abort();

        // Drain any remaining messages
        while let Ok(msg) = registry_rx.try_recv() {
            handle_registry_message(
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

async fn handle_registry_message(
    service_manager: &Arc<RwLock<ServiceManager>>,
    powersave_manager: &Arc<PowersaveManager>,
    state_change_tx: &mpsc::Sender<ServiceStateChange>,
    outbound_tx: &mpsc::Sender<AgentMessage>,
    msg: RegistryMessage,
) {
    match msg {
        RegistryMessage::Config { services, power_policy, base_domain, slug, frontend, apis, code_server_enabled, .. } => {
            info!("Received config from HomeRoute");

            // Update service manager config
            {
                let mut mgr = service_manager.write().unwrap();
                mgr.update_config(&services);
            }

            // Update power policy
            powersave_manager.set_policy(&power_policy);

            // Build and publish routes
            let mut routes = Vec::new();
            if let Some(ref fe) = frontend {
                routes.push(AgentRoute {
                    domain: format!("{}.{}", slug, base_domain),
                    target_port: fe.target_port,
                    service_type: ServiceType::App,
                    auth_required: fe.auth_required,
                    allowed_groups: fe.allowed_groups.clone(),
                });
            }
            for api in &apis {
                routes.push(AgentRoute {
                    domain: format!("{}-{}.{}", slug, api.slug, base_domain),
                    target_port: api.target_port,
                    service_type: ServiceType::App,
                    auth_required: api.auth_required,
                    allowed_groups: api.allowed_groups.clone(),
                });
            }
            if code_server_enabled {
                routes.push(AgentRoute {
                    domain: format!("{}.code.{}", slug, base_domain),
                    target_port: 13337,
                    service_type: ServiceType::CodeServer,
                    auth_required: true,
                    allowed_groups: vec![],
                });
            }
            if !routes.is_empty() {
                let routes_count = routes.len();
                let _ = outbound_tx.send(AgentMessage::PublishRoutes { routes }).await;
                info!("Published {} routes to HomeRoute", routes_count);
            }
        }

        RegistryMessage::Shutdown => {
            info!("Shutdown requested by HomeRoute");
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

            // Immediately send transitional state (Starting/Stopping) for instant UI feedback
            let transitional_state = match action {
                hr_registry::protocol::ServiceAction::Start => ServiceState::Starting,
                hr_registry::protocol::ServiceAction::Stop => ServiceState::Stopping,
            };
            let _ = outbound_tx
                .send(AgentMessage::ServiceStateChanged {
                    service_type,
                    new_state: transitional_state,
                })
                .await;

            // Execute the command
            powersave_manager.handle_command(service_type, action, state_change_tx).await;

            // Send final state after command completes
            let final_state = powersave_manager.get_state(service_type);
            let _ = outbound_tx
                .send(AgentMessage::ServiceStateChanged {
                    service_type,
                    new_state: final_state,
                })
                .await;
        }

        RegistryMessage::ActivityPing { service_type } => {
            powersave_manager.record_activity(service_type);
        }
    }
}
