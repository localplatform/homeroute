mod config;
mod connection;
mod dataverse;
mod mcp;
mod metrics;
mod powersave;
mod proxy;
mod services;
mod update;

use std::sync::{Arc, RwLock};

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{error, info};

use hr_registry::protocol::{AgentMessage, AgentMetrics, AgentRoute, RegistryMessage, ServiceConfig, ServiceState, ServiceType};

use crate::mcp::SchemaQuerySignals;
use crate::metrics::MetricsCollector;
use crate::powersave::{PowersaveManager, ServiceStateChange};
use crate::services::ServiceManager;

const CONFIG_PATH: &str = "/etc/hr-agent.toml";
const MAX_BACKOFF_SECS: u64 = 60;
const INITIAL_BACKOFF_SECS: u64 = 5;

#[tokio::main]
async fn main() -> Result<()> {
    // Check for MCP subcommands
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "mcp" {
        tracing_subscriber::fmt()
            .with_env_filter("warn")
            .with_writer(std::io::stderr)
            .init();

        // Try to establish a WebSocket connection to the registry for inter-app queries.
        // If the config or connection fails, run in standalone mode (no registry tools).
        match start_mcp_with_registry().await {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::warn!("Could not connect to registry for MCP: {e}, running standalone");
                return mcp::run_mcp_server().await;
            }
        }
    }

    if args.len() > 1 && args[1] == "mcp-deploy" {
        tracing_subscriber::fmt()
            .with_env_filter("warn")
            .with_writer(std::io::stderr)
            .init();

        return start_deploy_mcp().await;
    }

    if args.len() > 1 && args[1] == "mcp-store" {
        tracing_subscriber::fmt()
            .with_env_filter("warn")
            .with_writer(std::io::stderr)
            .init();

        return start_store_mcp().await;
    }

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

    // Open Dataverse database (shared across reconnections)
    let local_dataverse = match crate::dataverse::LocalDataverse::open() {
        Ok(dv) => {
            info!("Dataverse database opened");
            Some(Arc::new(dv))
        }
        Err(e) => {
            tracing::debug!("No Dataverse database available: {e}");
            None
        }
    };

    // Shared signal map for MCP → main loop schema query responses
    let schema_signals: SchemaQuerySignals =
        Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));

    // Agent HTTPS proxy (created once, survives reconnections)
    let agent_proxy: Arc<proxy::AgentProxy> = Arc::new(proxy::AgentProxy::new(&cfg));
    let mut proxy_started = false;

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

                let metrics = AgentMetrics {
                    code_server_status,
                    app_status,
                    db_status,
                    memory_bytes,
                    cpu_percent,
                    code_server_idle_secs: 0,
                };

                if metrics_tx.send(AgentMessage::Metrics(metrics)).await.is_err() {
                    // Channel closed, connection ended
                    break;
                }
            }
        });

        // Spawn schema metadata sender task (every 60 seconds)
        let schema_tx = outbound_tx.clone();
        let schema_dv = local_dataverse.clone();
        let schema_handle = tokio::spawn(async move {
            let Some(dv) = schema_dv else { return };
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                match dv.get_schema_metadata().await {
                    Ok((tables, relations, version, db_size_bytes)) => {
                        if schema_tx
                            .send(AgentMessage::SchemaMetadata {
                                tables,
                                relations,
                                version,
                                db_size_bytes,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Failed to get schema metadata: {e}");
                    }
                }
            }
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
                                &local_dataverse,
                                &schema_signals,
                                &agent_proxy,
                                &mut proxy_started,
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
        schema_handle.abort();

        // Drain any remaining messages
        while let Ok(msg) = registry_rx.try_recv() {
            handle_registry_message(
                &service_manager,
                &powersave_manager,
                &state_change_tx,
                &outbound_tx,
                &local_dataverse,
                &schema_signals,
                &agent_proxy,
                &mut proxy_started,
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

/// Start the MCP server with a WebSocket connection to the registry.
/// This allows the `list_other_apps_schemas` tool to query other apps' schemas.
async fn start_mcp_with_registry() -> Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let cfg = config::AgentConfig::load(CONFIG_PATH)?;
    let url = cfg.ws_url();

    let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await?;
    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // Authenticate
    let auth_msg = AgentMessage::Auth {
        token: cfg.token.clone(),
        service_name: cfg.service_name.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        ipv4_address: None,
    };
    ws_sink
        .send(Message::Text(serde_json::to_string(&auth_msg)?.into()))
        .await?;

    // Wait for auth result
    let first_msg = ws_stream
        .next()
        .await
        .ok_or_else(|| anyhow::anyhow!("Connection closed before auth response"))??;
    let auth_result: RegistryMessage = match first_msg {
        Message::Text(text) => serde_json::from_str(&text)?,
        other => anyhow::bail!("Unexpected message type during auth: {other:?}"),
    };
    match auth_result {
        RegistryMessage::AuthResult { success: true, .. } => {}
        RegistryMessage::AuthResult { success: false, error, .. } => {
            anyhow::bail!(
                "Authentication failed: {}",
                error.unwrap_or_default()
            );
        }
        _ => anyhow::bail!("Unexpected message during auth handshake"),
    };

    // Set up the channels
    let schema_signals: mcp::SchemaQuerySignals =
        Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<AgentMessage>(16);

    // Background task: forward outbound messages to the WebSocket
    let ws_write_handle = tokio::spawn(async move {
        while let Some(msg) = outbound_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                if ws_sink.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    // Background task: read WebSocket messages and dispatch DataverseSchemas responses
    let signals_clone = Arc::clone(&schema_signals);
    let ws_read_handle = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_stream.next().await {
            if let Message::Text(text) = msg {
                if let Ok(registry_msg) = serde_json::from_str::<RegistryMessage>(&text) {
                    if let RegistryMessage::DataverseSchemas {
                        request_id,
                        schemas,
                    } = registry_msg
                    {
                        let sender = {
                            let mut signals = signals_clone.write().await;
                            signals.remove(&request_id)
                        };
                        if let Some(tx) = sender {
                            let _ = tx.send(schemas);
                        }
                    }
                    // Ignore all other messages in MCP mode
                }
            }
        }
    });

    // Run the MCP stdio server with registry access (Dataverse only)
    let result = mcp::run_mcp_server_with_registry(Some(outbound_tx), Some(schema_signals)).await;

    // Clean up background tasks
    ws_write_handle.abort();
    ws_read_handle.abort();

    result
}

/// Start the Deploy MCP server — connects to registry to get app_id and environment.
async fn start_deploy_mcp() -> Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let cfg = config::AgentConfig::load(CONFIG_PATH)?;
    let url = cfg.ws_url();

    let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await?;
    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // Authenticate
    let auth_msg = AgentMessage::Auth {
        token: cfg.token.clone(),
        service_name: cfg.service_name.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        ipv4_address: None,
    };
    ws_sink
        .send(Message::Text(serde_json::to_string(&auth_msg)?.into()))
        .await?;

    // Wait for auth result
    let first_msg = ws_stream
        .next()
        .await
        .ok_or_else(|| anyhow::anyhow!("Connection closed before auth response"))??;
    let auth_result: RegistryMessage = match first_msg {
        Message::Text(text) => serde_json::from_str(&text)?,
        other => anyhow::bail!("Unexpected message type during auth: {other:?}"),
    };
    let app_id = match auth_result {
        RegistryMessage::AuthResult { success: true, app_id, .. } => {
            app_id.unwrap_or_default()
        }
        RegistryMessage::AuthResult { success: false, error, .. } => {
            anyhow::bail!("Authentication failed: {}", error.unwrap_or_default());
        }
        _ => anyhow::bail!("Unexpected message during auth handshake"),
    };

    // Read the Config message to get the environment
    let mut environment = hr_registry::types::Environment::Development;
    if let Ok(Some(Ok(msg))) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ws_stream.next(),
    ).await {
        if let Message::Text(text) = msg {
            if let Ok(RegistryMessage::Config { environment: env, .. }) = serde_json::from_str::<RegistryMessage>(&text) {
                environment = env;
            }
        }
    }

    // Build deploy context
    let api_base_url = format!("http://{}:{}", cfg.homeroute_address, cfg.homeroute_port);
    let deploy_ctx = mcp::DeployContext {
        app_id,
        api_base_url,
        environment,
    };

    // Close the WebSocket (deploy MCP doesn't need ongoing WS connection)
    drop(ws_sink);

    mcp::run_deploy_mcp_server(deploy_ctx).await
}

/// Start the Store MCP server — connects to registry to get app_id.
async fn start_store_mcp() -> Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let cfg = config::AgentConfig::load(CONFIG_PATH)?;
    let url = cfg.ws_url();

    let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await?;
    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // Authenticate
    let auth_msg = AgentMessage::Auth {
        token: cfg.token.clone(),
        service_name: cfg.service_name.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        ipv4_address: None,
    };
    ws_sink
        .send(Message::Text(serde_json::to_string(&auth_msg)?.into()))
        .await?;

    // Wait for auth result
    let first_msg = ws_stream
        .next()
        .await
        .ok_or_else(|| anyhow::anyhow!("Connection closed before auth response"))??;
    let auth_result: RegistryMessage = match first_msg {
        Message::Text(text) => serde_json::from_str(&text)?,
        other => anyhow::bail!("Unexpected message type during auth: {other:?}"),
    };
    let app_id = match auth_result {
        RegistryMessage::AuthResult { success: true, app_id, .. } => {
            app_id.unwrap_or_default()
        }
        RegistryMessage::AuthResult { success: false, error, .. } => {
            anyhow::bail!("Authentication failed: {}", error.unwrap_or_default());
        }
        _ => anyhow::bail!("Unexpected message during auth handshake"),
    };

    // Build store context
    let api_base_url = format!("http://{}:{}", cfg.homeroute_address, cfg.homeroute_port);
    let ctx = mcp::StoreContext {
        app_id,
        api_base_url,
    };

    // Close the WebSocket (store MCP doesn't need ongoing WS connection)
    drop(ws_sink);

    mcp::run_store_mcp_server(ctx).await
}

async fn handle_registry_message(
    service_manager: &Arc<RwLock<ServiceManager>>,
    powersave_manager: &Arc<PowersaveManager>,
    state_change_tx: &mpsc::Sender<ServiceStateChange>,
    outbound_tx: &mpsc::Sender<AgentMessage>,
    local_dataverse: &Option<Arc<crate::dataverse::LocalDataverse>>,
    schema_signals: &SchemaQuerySignals,
    agent_proxy: &Arc<proxy::AgentProxy>,
    proxy_started: &mut bool,
    msg: RegistryMessage,
) {
    match msg {
        RegistryMessage::Config { services, base_domain, slug, frontend, environment, code_server_enabled, .. } => {
            info!("Received config from HomeRoute");

            // Update service manager config
            {
                let mut mgr = service_manager.write().unwrap();
                mgr.update_config(&services);
            }

            // Write/update .mcp.json for MCP tool discovery
            let is_dev = matches!(environment, hr_registry::types::Environment::Development);
            let workspace = std::path::Path::new("/root/workspace");
            if workspace.is_dir() {
                let content = mcp::generate_mcp_json(is_dev);
                match std::fs::write(workspace.join(".mcp.json"), &content) {
                    Ok(()) => info!("Updated /root/workspace/.mcp.json"),
                    Err(e) => tracing::debug!("Could not write .mcp.json: {e}"),
                }
            }

            // Update agent proxy route table
            agent_proxy.update_routes(
                &base_domain,
                &slug,
                frontend.as_ref(),
                environment,
                code_server_enabled,
            );

            // On first Config: pull certs and start the HTTPS proxy
            if !*proxy_started {
                match agent_proxy.update_certs().await {
                    Ok(()) => {
                        agent_proxy.start();
                        *proxy_started = true;
                        info!("Agent HTTPS proxy started");
                    }
                    Err(e) => {
                        error!("Failed to pull initial certs, proxy NOT started: {e}");
                    }
                }
            }

            // Build and publish routes based on environment:
            // Dev: code.{slug}.{base} (if code_server_enabled)
            // Prod: {slug}.{base}
            // All routes point to port 443 (agent proxy handles internal routing)
            let mut routes = Vec::new();
            match environment {
                hr_registry::types::Environment::Development => {
                    if code_server_enabled {
                        routes.push(AgentRoute {
                            domain: format!("code.{}.{}", slug, base_domain),
                            target_port: 443,
                            service_type: ServiceType::CodeServer,
                            auth_required: false,
                            allowed_groups: vec![],
                        });
                    }
                }
                hr_registry::types::Environment::Production => {
                    if frontend.is_some() {
                        routes.push(AgentRoute {
                            domain: format!("{}.{}", slug, base_domain),
                            target_port: 443,
                            service_type: ServiceType::App,
                            auth_required: false,
                            allowed_groups: vec![],
                        });
                    }
                }
            }
            if !routes.is_empty() {
                let routes_count = routes.len();
                let _ = outbound_tx.send(AgentMessage::PublishRoutes { routes }).await;
                info!("Published {} routes to HomeRoute (all port 443)", routes_count);
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

        RegistryMessage::DataverseQuery { request_id, query } => {
            let result = if let Some(dv) = local_dataverse {
                handle_dataverse_query(dv, query).await
            } else {
                Err("Dataverse not available on this agent".to_string())
            };
            let (data, error) = match result {
                Ok(v) => (Some(v), None),
                Err(e) => (None, Some(e)),
            };
            let _ = outbound_tx
                .send(AgentMessage::DataverseQueryResult { request_id, data, error })
                .await;
        }

        RegistryMessage::DataverseSchemas { request_id, schemas } => {
            // Resolve the pending oneshot sender for the MCP tool waiting on this response
            let sender = {
                let mut signals = schema_signals.write().await;
                signals.remove(&request_id)
            };
            if let Some(tx) = sender {
                let _ = tx.send(schemas);
            } else {
                tracing::debug!(request_id, "No pending signal for DataverseSchemas response");
            }
        }

        RegistryMessage::CertRenewal { slug } => {
            info!(slug, "Certificate renewal notification, re-pulling certs");
            let proxy = Arc::clone(agent_proxy);
            tokio::spawn(async move {
                if let Err(e) = proxy.update_certs().await {
                    error!("Failed to update certs after renewal: {e}");
                }
            });
        }

        _ => {}
    }
}

async fn handle_dataverse_query(
    dv: &Arc<crate::dataverse::LocalDataverse>,
    query: hr_registry::protocol::DataverseQueryRequest,
) -> Result<serde_json::Value, String> {
    use hr_dataverse::query::*;
    use hr_registry::protocol::DataverseQueryRequest;

    let engine = dv.engine().lock().await;

    match query {
        DataverseQueryRequest::QueryRows { table_name, filters, limit, offset, order_by, order_desc } => {
            let parsed_filters: Vec<Filter> = filters.iter()
                .filter_map(|f| serde_json::from_value(f.clone()).ok())
                .collect();
            let pagination = Pagination {
                limit,
                offset,
                order_by,
                order_desc,
            };
            let rows = query_rows(engine.connection(), &table_name, &parsed_filters, &pagination)
                .map_err(|e| e.to_string())?;
            let total = engine.count_rows(&table_name).unwrap_or(0);
            Ok(serde_json::json!({ "rows": rows, "total": total }))
        }
        DataverseQueryRequest::InsertRows { table_name, rows } => {
            let count = insert_rows(engine.connection(), &table_name, &rows)
                .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "inserted": count }))
        }
        DataverseQueryRequest::UpdateRows { table_name, updates, filters } => {
            let parsed_filters: Vec<Filter> = filters.iter()
                .filter_map(|f| serde_json::from_value(f.clone()).ok())
                .collect();
            let count = update_rows(engine.connection(), &table_name, &updates, &parsed_filters)
                .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "updated": count }))
        }
        DataverseQueryRequest::DeleteRows { table_name, filters } => {
            let parsed_filters: Vec<Filter> = filters.iter()
                .filter_map(|f| serde_json::from_value(f.clone()).ok())
                .collect();
            let count = delete_rows(engine.connection(), &table_name, &parsed_filters)
                .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "deleted": count }))
        }
        DataverseQueryRequest::CountRows { table_name, filters } => {
            if filters.is_empty() {
                let count = engine.count_rows(&table_name).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "count": count }))
            } else {
                let parsed_filters: Vec<Filter> = filters.iter()
                    .filter_map(|f| serde_json::from_value(f.clone()).ok())
                    .collect();
                let pagination = Pagination { limit: 0, offset: 0, order_by: None, order_desc: false };
                // Use a COUNT query by counting filtered results
                let rows = query_rows(engine.connection(), &table_name, &parsed_filters, &Pagination { limit: u64::MAX, ..pagination })
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "count": rows.len() }))
            }
        }
        DataverseQueryRequest::GetMigrations => {
            let rows = query_rows(
                engine.connection(),
                "_dv_migrations",
                &[],
                &Pagination { limit: 1000, offset: 0, order_by: Some("id".to_string()), order_desc: true },
            ).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "migrations": rows }))
        }
    }
}
