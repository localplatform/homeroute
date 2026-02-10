//! REST API + WebSocket routes for application management.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use tokio::io::AsyncReadExt;
use tracing::{error, info, warn};

use hr_proxy::AppRoute;
use hr_registry::protocol::{AgentMessage, HostRegistryMessage, PowerPolicy, ServiceAction, ServiceType};
use hr_registry::types::TriggerUpdateRequest;
use hr_common::events::{MigrationPhase, MigrationProgressEvent};
use hr_acme::types::WildcardType;
use hr_dns::config::StaticRecord;

use crate::state::{ApiState, MigrationState};

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/{id}/services/{service_type}/start", post(start_service))
        .route("/{id}/services/{service_type}/stop", post(stop_service))
        .route("/{id}/power-policy", put(update_power_policy))
        .route("/{id}/update/fix", post(fix_agent_update))
        .route("/agents/version", get(agent_version))
        .route("/agents/binary", get(agent_binary))
        .route("/agents/certs", get(agent_certs))
        .route("/agents/update", post(trigger_agent_update))
        .route("/agents/update/status", get(get_update_status))
        .route("/agents/ws", get(agent_ws))
}

// ── REST handlers ────────────────────────────────────────────

async fn start_service(
    State(state): State<ApiState>,
    Path((id, service_type_str)): Path<(String, String)>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    let service_type = match service_type_str.as_str() {
        "code-server" => ServiceType::CodeServer,
        "app" => ServiceType::App,
        "db" => ServiceType::Db,
        _ => {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"success": false, "error": "Invalid service type"}))).into_response();
        }
    };

    match registry.send_service_command(&id, service_type, ServiceAction::Start).await {
        Ok(true) => {
            info!(app_id = id, service = service_type_str, "Service start command sent");
            Json(serde_json::json!({"success": true})).into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"success": false, "error": "Application not found or not connected"}))).into_response(),
        Err(e) => {
            error!("Failed to send start command: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"success": false, "error": e.to_string()}))).into_response()
        }
    }
}

async fn stop_service(
    State(state): State<ApiState>,
    Path((id, service_type_str)): Path<(String, String)>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    let service_type = match service_type_str.as_str() {
        "code-server" => ServiceType::CodeServer,
        "app" => ServiceType::App,
        "db" => ServiceType::Db,
        _ => {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"success": false, "error": "Invalid service type"}))).into_response();
        }
    };

    match registry.send_service_command(&id, service_type, ServiceAction::Stop).await {
        Ok(true) => {
            info!(app_id = id, service = service_type_str, "Service stop command sent");
            Json(serde_json::json!({"success": true})).into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"success": false, "error": "Application not found or not connected"}))).into_response(),
        Err(e) => {
            error!("Failed to send stop command: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"success": false, "error": e.to_string()}))).into_response()
        }
    }
}

async fn update_power_policy(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(policy): Json<PowerPolicy>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    match registry.update_power_policy(&id, policy).await {
        Ok(true) => {
            info!(app_id = id, "Power policy updated");
            Json(serde_json::json!({"success": true})).into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"success": false, "error": "Application not found"}))).into_response(),
        Err(e) => {
            error!("Failed to update power policy: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"success": false, "error": e.to_string()}))).into_response()
        }
    }
}

// ── Agent update handlers ────────────────────────────────────

/// Trigger update to all connected agents (or specific ones).
async fn trigger_agent_update(
    State(state): State<ApiState>,
    Json(req): Json<TriggerUpdateRequest>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"success": false, "error": "Registry not available"})),
        )
            .into_response();
    };

    match registry.trigger_update(req.agent_ids).await {
        Ok(result) => {
            info!(
                notified = result.agents_notified.len(),
                skipped = result.agents_skipped.len(),
                version = result.version,
                "Agent update triggered via API"
            );
            Json(serde_json::json!({
                "success": true,
                "version": result.version,
                "sha256": result.sha256,
                "agents_notified": result.agents_notified,
                "agents_skipped": result.agents_skipped
            }))
            .into_response()
        }
        Err(e) => {
            error!("Failed to trigger agent update: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"success": false, "error": e.to_string()})),
            )
                .into_response()
        }
    }
}

/// Get update status for all agents.
async fn get_update_status(State(state): State<ApiState>) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"success": false, "error": "Registry not available"})),
        )
            .into_response();
    };

    match registry.get_update_status().await {
        Ok(result) => Json(serde_json::json!({
            "success": true,
            "expected_version": result.expected_version,
            "agents": result.agents
        }))
        .into_response(),
        Err(e) => {
            error!("Failed to get update status: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"success": false, "error": e.to_string()})),
            )
                .into_response()
        }
    }
}

/// Fix a failed agent update via machinectl exec (local) or remote exec (remote host).
async fn fix_agent_update(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"success": false, "error": "Registry not available"})),
        )
            .into_response();
    };

    // Look up the app to determine if local or remote
    let app = registry.get_application(&id).await;
    match app {
        Some(app) if app.host_id == "local" => {
            match registry.fix_agent_via_exec(&id).await {
                Ok(output) => {
                    info!(app_id = id, "Agent fixed via machinectl exec");
                    Json(serde_json::json!({"success": true, "output": output})).into_response()
                }
                Err(e) => {
                    error!(app_id = id, "Failed to fix agent: {e}");
                    (StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"success": false, "error": e.to_string()}))).into_response()
                }
            }
        }
        Some(app) => {
            let api_port = state.env.api_port;
            let cmd = vec![
                "bash".to_string(), "-c".to_string(),
                format!(
                    "curl -fsSL http://10.0.0.254:{}/api/applications/agents/binary -o /usr/local/bin/hr-agent.new && \
                     chmod +x /usr/local/bin/hr-agent.new && \
                     mv /usr/local/bin/hr-agent.new /usr/local/bin/hr-agent && \
                     systemctl restart hr-agent",
                    api_port
                ),
            ];
            match registry.exec_in_remote_container(&app.host_id, &app.container_name, cmd).await {
                Ok((true, stdout, _)) => {
                    info!(app_id = id, "Agent fixed via remote exec");
                    Json(serde_json::json!({"success": true, "output": stdout})).into_response()
                }
                Ok((false, _, stderr)) => {
                    error!(app_id = id, "Remote fix failed: {}", stderr);
                    (StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"success": false, "error": stderr}))).into_response()
                }
                Err(e) => {
                    error!(app_id = id, "Remote exec failed: {e}");
                    (StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"success": false, "error": e.to_string()}))).into_response()
                }
            }
        }
        None => {
            (StatusCode::NOT_FOUND,
                Json(serde_json::json!({"success": false, "error": "Application not found"}))).into_response()
        }
    }
}

// ── Agent binary distribution ────────────────────────────────

const AGENT_BINARY_PATH: &str = "/opt/homeroute/data/agent-binaries/hr-agent";

async fn agent_version() -> impl IntoResponse {
    let binary_path = std::path::Path::new(AGENT_BINARY_PATH);
    if !binary_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"success": false, "error": "Agent binary not found"})),
        )
            .into_response();
    }

    // Read binary and compute SHA256
    let bytes = match tokio::fs::read(binary_path).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"success": false, "error": e.to_string()})),
            )
                .into_response();
        }
    };

    let digest = ring::digest::digest(&ring::digest::SHA256, &bytes);
    let sha256: String = digest.as_ref().iter().map(|b| format!("{:02x}", b)).collect();

    // Version from the binary metadata (or file mtime as fallback)
    let version = match tokio::fs::metadata(binary_path).await {
        Ok(m) => {
            if let Ok(modified) = m.modified() {
                let dt: chrono::DateTime<chrono::Utc> = modified.into();
                dt.format("%Y%m%d-%H%M%S").to_string()
            } else {
                "unknown".to_string()
            }
        }
        Err(_) => "unknown".to_string(),
    };

    Json(serde_json::json!({
        "success": true,
        "version": version,
        "sha256": sha256,
        "size": bytes.len()
    }))
    .into_response()
}

async fn agent_binary() -> impl IntoResponse {
    let binary_path = std::path::Path::new(AGENT_BINARY_PATH);
    match tokio::fs::read(binary_path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/octet-stream"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"hr-agent\"",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent binary not found"})),
        )
            .into_response(),
    }
}

// ── Agent certificate distribution ───────────────────────────

/// GET /api/applications/agents/certs
/// Auth via `Authorization: Bearer {agent_token}` header.
/// Returns cert+key PEM for the app wildcard and global wildcard.
async fn agent_certs(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": "Registry not available"}))).into_response();
    };

    // Extract Bearer token
    let token = match headers.get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        Some(t) => t,
        None => {
            return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "Missing or invalid Authorization header"}))).into_response();
        }
    };

    // Authenticate by token (tries all applications)
    let (app_id, slug) = match registry.authenticate_by_token(token).await {
        Some(v) => v,
        None => {
            return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "Invalid token"}))).into_response();
        }
    };

    info!(app_id, slug, "Agent fetching certificates");

    // Get app-specific wildcard cert
    let app_cert = match state.acme.get_cert_pem(WildcardType::App { slug: slug.clone() }).await {
        Ok((cert_pem, key_pem)) => {
            let wildcard_domain = WildcardType::App { slug: slug.clone() }.domain_pattern(&state.env.base_domain);
            Some(serde_json::json!({
                "cert_pem": cert_pem,
                "key_pem": key_pem,
                "wildcard_domain": wildcard_domain,
            }))
        }
        Err(_) => None,
    };

    // Get global wildcard cert
    let global_cert = match state.acme.get_cert_pem(WildcardType::Global).await {
        Ok((cert_pem, key_pem)) => {
            let wildcard_domain = WildcardType::Global.domain_pattern(&state.env.base_domain);
            Some(serde_json::json!({
                "cert_pem": cert_pem,
                "key_pem": key_pem,
                "wildcard_domain": wildcard_domain,
            }))
        }
        Err(_) => None,
    };

    Json(serde_json::json!({
        "app_cert": app_cert,
        "global_cert": global_cert,
    })).into_response()
}

// ── DNS record helpers for agent lifecycle ───────────────────

/// Add local DNS A records for an agent: `{slug}.{base}` and `*.{slug}.{base}` → IPv4.
async fn add_agent_dns_records(dns: &hr_dns::SharedDnsState, slug: &str, base_domain: &str, ipv4: &str) {
    let mut dns_state = dns.write().await;
    dns_state.add_static_record(StaticRecord {
        name: format!("{}.{}", slug, base_domain),
        record_type: "A".to_string(),
        value: ipv4.to_string(),
        ttl: 60,
    });
    dns_state.add_static_record(StaticRecord {
        name: format!("*.{}.{}", slug, base_domain),
        record_type: "A".to_string(),
        value: ipv4.to_string(),
        ttl: 60,
    });
    info!(slug, ipv4, "Added local DNS A records for agent");
}

/// Remove all local DNS records pointing to a specific IPv4 address.
async fn remove_agent_dns_records(dns: &hr_dns::SharedDnsState, ipv4: &str) {
    let mut dns_state = dns.write().await;
    dns_state.remove_static_records_by_value(ipv4);
    info!(ipv4, "Removed local DNS records for agent IP");
}

// ── WebSocket handler for agent connections ─────────────────

async fn agent_ws(
    State(state): State<ApiState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_agent_ws(state, socket))
}

async fn handle_agent_ws(state: ApiState, mut socket: WebSocket) {
    let Some(registry) = &state.registry else {
        let _ = socket.send(Message::Close(None)).await;
        return;
    };
    let registry = registry.clone();

    // Wait for Auth message with a timeout
    let auth_msg = tokio::time::timeout(std::time::Duration::from_secs(5), socket.recv()).await;

    let (token, service_name, version, reported_ipv4) = match auth_msg {
        Ok(Some(Ok(Message::Text(text)))) => {
            match serde_json::from_str::<AgentMessage>(&text) {
                Ok(AgentMessage::Auth { token, service_name, version, ipv4_address }) => {
                    (token, service_name, version, ipv4_address)
                }
                _ => {
                    warn!("Agent WS: expected Auth message, got something else");
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            }
        }
        _ => {
            warn!("Agent WS: auth timeout or connection error");
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };

    // Authenticate
    let Some(app_id) = registry.authenticate(&token, &service_name).await else {
        let reject = hr_registry::protocol::RegistryMessage::AuthResult {
            success: false,
            error: Some("Invalid credentials".into()),
        };
        let _ = socket.send(Message::Text(serde_json::to_string(&reject).unwrap().into())).await;
        let _ = socket.send(Message::Close(None)).await;
        return;
    };

    info!(app_id = app_id, service = service_name, ipv4 = ?reported_ipv4, "Agent authenticated");

    // Create mpsc channel for registry → agent messages
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);

    // Notify registry of connection (pushes config, increments active count)
    if let Err(e) = registry.on_agent_connected(&app_id, tx, version, reported_ipv4).await {
        error!(app_id, "Agent provisioning failed: {e}");
        // Decrement the count that was already incremented
        registry.on_agent_disconnected(&app_id).await;
        let _ = socket.send(Message::Close(None)).await;
        return;
    }

    // Routes are now published by the agent via PublishRoutes message.

    // Send auth success
    let success = hr_registry::protocol::RegistryMessage::AuthResult {
        success: true,
        error: None,
    };
    if socket.send(Message::Text(serde_json::to_string(&success).unwrap().into())).await.is_err() {
        registry.on_agent_disconnected(&app_id).await;
        return;
    }

    // Bidirectional message loop
    loop {
        tokio::select! {
            // Registry → Agent
            Some(msg) = rx.recv() => {
                let json = match serde_json::to_string(&msg) {
                    Ok(j) => j,
                    Err(_) => continue,
                };
                if socket.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            // Agent → Registry
            ws_msg = socket.recv() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<AgentMessage>(&text) {
                            Ok(AgentMessage::Heartbeat { .. }) => {
                                registry.handle_heartbeat(&app_id).await;
                            }
                            Ok(AgentMessage::ConfigAck { .. }) => {
                                // Acknowledged, nothing to do
                            }
                            Ok(AgentMessage::Error { message }) => {
                                warn!(app_id, message, "Agent reported error");
                            }
                            Ok(AgentMessage::Auth { .. }) => {
                                // Duplicate auth, ignore
                            }
                            Ok(AgentMessage::Metrics(m)) => {
                                // Metrics are proof of liveness — update heartbeat
                                // (restores Connected status after host suspend/resume)
                                registry.handle_heartbeat(&app_id).await;
                                registry.handle_metrics(&app_id, m).await;
                            }
                            Ok(AgentMessage::ServiceStateChanged { service_type, new_state }) => {
                                info!(
                                    app_id,
                                    service_type = ?service_type,
                                    new_state = ?new_state,
                                    "Agent reported service state change"
                                );
                                // Broadcast to WebSocket clients
                                registry.handle_service_state_changed(&app_id, service_type, new_state);
                            }
                            Ok(AgentMessage::SchemaMetadata { tables, relations, version, db_size_bytes }) => {
                                info!(app_id, tables = tables.len(), version, "Agent reported schema metadata");
                                registry.handle_schema_metadata(&app_id, tables.clone(), relations.clone(), version, db_size_bytes).await;

                                // Update the Dataverse schema cache in ApiState
                                let slug = registry.list_applications().await
                                    .iter()
                                    .find(|a| a.id == app_id)
                                    .map(|a| a.slug.clone())
                                    .unwrap_or_default();
                                let cached = crate::state::CachedDataverseSchema {
                                    app_id: app_id.clone(),
                                    slug,
                                    tables: tables.iter().map(|t| crate::state::CachedTableInfo {
                                        name: t.name.clone(),
                                        slug: t.slug.clone(),
                                        columns: t.columns.iter().map(|c| crate::state::CachedColumnInfo {
                                            name: c.name.clone(),
                                            field_type: c.field_type.clone(),
                                            required: c.required,
                                            unique: c.unique,
                                        }).collect(),
                                        row_count: t.row_count,
                                    }).collect(),
                                    relations: relations.iter().map(|r| crate::state::CachedRelationInfo {
                                        from_table: r.from_table.clone(),
                                        from_column: r.from_column.clone(),
                                        to_table: r.to_table.clone(),
                                        to_column: r.to_column.clone(),
                                        relation_type: r.relation_type.clone(),
                                    }).collect(),
                                    version,
                                    db_size_bytes,
                                    last_updated: chrono::Utc::now(),
                                };
                                state.dataverse_schemas.write().await.insert(app_id.clone(), cached);
                            }
                            Ok(AgentMessage::DataverseQueryResult { request_id, data, error }) => {
                                registry.on_dataverse_query_result(&request_id, data, error).await;
                            }
                            Ok(AgentMessage::GetDataverseSchemas { request_id }) => {
                                // Build schema overviews from the cached data in ApiState
                                use hr_registry::protocol::{AppSchemaOverview, SchemaTableInfo, SchemaColumnInfo, SchemaRelationInfo};
                                let schemas = state.dataverse_schemas.read().await;
                                let overviews: Vec<AppSchemaOverview> = schemas.values()
                                    .filter(|s| s.app_id != app_id)
                                    .map(|s| AppSchemaOverview {
                                        app_id: s.app_id.clone(),
                                        slug: s.slug.clone(),
                                        tables: s.tables.iter().map(|t| SchemaTableInfo {
                                            name: t.name.clone(),
                                            slug: t.slug.clone(),
                                            columns: t.columns.iter().map(|c| SchemaColumnInfo {
                                                name: c.name.clone(),
                                                field_type: c.field_type.clone(),
                                                required: c.required,
                                                unique: c.unique,
                                            }).collect(),
                                            row_count: t.row_count,
                                        }).collect(),
                                        relations: s.relations.iter().map(|r| SchemaRelationInfo {
                                            from_table: r.from_table.clone(),
                                            from_column: r.from_column.clone(),
                                            to_table: r.to_table.clone(),
                                            to_column: r.to_column.clone(),
                                            relation_type: r.relation_type.clone(),
                                        }).collect(),
                                        version: s.version,
                                    })
                                    .collect();
                                let _ = registry.send_to_agent(&app_id, hr_registry::protocol::RegistryMessage::DataverseSchemas {
                                    request_id,
                                    schemas: overviews,
                                }).await;
                            }
                            Ok(AgentMessage::IpUpdate { ipv4_address }) => {
                                info!(app_id, ipv4_address, "Agent reported IP update");
                                // Remove old DNS records for previous IP
                                if let Some(app) = registry.get_application(&app_id).await {
                                    if let Some(old_ip) = app.ipv4_address {
                                        remove_agent_dns_records(&state.dns, &old_ip.to_string()).await;
                                    }
                                }
                                registry.handle_ip_update(&app_id, &ipv4_address).await;
                                // Add new DNS records for updated IP
                                if let Some(app) = registry.get_application(&app_id).await {
                                    add_agent_dns_records(&state.dns, &app.slug, &state.env.base_domain, &ipv4_address).await;
                                }
                            }
                            Ok(AgentMessage::PublishRoutes { routes }) => {
                                info!(app_id, count = routes.len(), "Agent published routes");
                                let apps = registry.list_applications().await;
                                if let Some(app) = apps.iter().find(|a| a.id == app_id) {
                                    if let Some(target_ip) = app.ipv4_address {
                                        // Clear old routes for this app
                                        let base_domain = &state.env.base_domain;
                                        for domain in app.domains(base_domain) {
                                            state.proxy.remove_app_route(&domain);
                                        }
                                        // Set new routes from agent
                                        for route in &routes {
                                            state.proxy.set_app_route(route.domain.clone(), AppRoute {
                                                app_id: app.id.clone(),
                                                host_id: app.host_id.clone(),
                                                target_ip,
                                                target_port: route.target_port,
                                                auth_required: route.auth_required,
                                                allowed_groups: route.allowed_groups.clone(),
                                                service_type: route.service_type,
                                                wake_page_enabled: app.wake_page_enabled,
                                            });
                                        }
                                        // Add local DNS A records for direct local access
                                        let ip_str = target_ip.to_string();
                                        add_agent_dns_records(&state.dns, &app.slug, base_domain, &ip_str).await;
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(app_id, "Invalid agent message: {e}");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    // Decrement connection count. Only remove routes when the LAST connection closes.
    let is_last = registry.on_agent_disconnected(&app_id).await;
    if is_last {
        let apps = registry.list_applications().await;
        if let Some(app) = apps.iter().find(|a| a.id == app_id) {
            let base_domain = &state.env.base_domain;
            for domain in app.domains(base_domain) {
                state.proxy.remove_app_route(&domain);
            }
            // Remove local DNS A records for this agent
            if let Some(ip) = app.ipv4_address {
                remove_agent_dns_records(&state.dns, &ip.to_string()).await;
            }
        }
        info!(app_id, "Agent WebSocket closed (last connection, routes + DNS removed)");
    } else {
        info!(app_id, "Agent WebSocket closed (other connections still active, routes preserved)");
    }
}

// ── Migration orchestration ──────────────────────────────────

// Helper to update migration state and emit event
pub(crate) async fn update_migration_phase(
    migrations: &Arc<tokio::sync::RwLock<std::collections::HashMap<String, MigrationState>>>,
    events: &Arc<hr_common::events::EventBus>,
    app_id: &str,
    transfer_id: &str,
    phase: MigrationPhase,
    pct: u8,
    transferred: u64,
    total: u64,
    error: Option<String>,
) {
    {
        let mut m = migrations.write().await;
        if let Some(state) = m.get_mut(transfer_id) {
            state.phase = phase.clone();
            state.progress_pct = pct;
            state.bytes_transferred = transferred;
            state.total_bytes = total;
            state.error = error.clone();
        }
    }
    let _ = events.migration_progress.send(MigrationProgressEvent {
        app_id: app_id.to_string(),
        transfer_id: transfer_id.to_string(),
        phase,
        progress_pct: pct,
        bytes_transferred: transferred,
        total_bytes: total,
        error,
    });
}

/// Stream data from an AsyncRead source to a remote host-agent in 512KB binary chunks.
/// Returns total bytes transferred and final sequence number.
pub(crate) async fn stream_to_remote(
    registry: &Arc<hr_registry::AgentRegistry>,
    target_host_id: &str,
    transfer_id: &str,
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
    total_bytes: u64,
    cancelled: &Arc<AtomicBool>,
    migrations: &Arc<tokio::sync::RwLock<std::collections::HashMap<String, MigrationState>>>,
    events: &Arc<hr_common::events::EventBus>,
    app_id: &str,
    pct_start: u8,
    pct_end: u8,
    phase: MigrationPhase,
) -> Result<(u64, u32), String> {
    let mut buf = vec![0u8; 524288]; // 512KB
    let mut transferred: u64 = 0;
    let mut sequence: u32 = 0;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            let _ = registry.send_host_command(
                target_host_id,
                HostRegistryMessage::CancelTransfer { transfer_id: transfer_id.to_string() },
            ).await;
            return Err("Migration cancelled by user".to_string());
        }

        let n = match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Err(format!("Read error: {e}")),
        };

        let chunk = &buf[..n];
        let checksum = xxhash_rust::xxh32::xxh32(chunk, 0);

        if let Err(e) = registry.send_host_command(
            target_host_id,
            HostRegistryMessage::ReceiveChunkBinary {
                transfer_id: transfer_id.to_string(),
                sequence,
                size: n as u32,
                checksum,
            },
        ).await {
            return Err(format!("Send chunk metadata failed: {e}"));
        }

        if let Err(e) = registry.send_host_binary(
            target_host_id,
            chunk.to_vec(),
        ).await {
            return Err(format!("Send binary chunk failed: {e}"));
        }

        transferred += n as u64;
        sequence += 1;
        let pct = (pct_start as u64 + (transferred * (pct_end - pct_start) as u64 / total_bytes.max(1))) as u8;

        if sequence % 4 == 0 || transferred >= total_bytes {
            update_migration_phase(migrations, events, app_id, transfer_id, phase.clone(), pct.min(pct_end), transferred, total_bytes, None).await;
        } else {
            let mut m = migrations.write().await;
            if let Some(state) = m.get_mut(transfer_id) {
                state.progress_pct = pct.min(pct_end);
                state.bytes_transferred = transferred;
            }
        }
    }

    Ok((transferred, sequence))
}

// Inter-host nspawn migration is in container_manager.rs
