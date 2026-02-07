//! REST API + WebSocket routes for application management.

use std::sync::Arc;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tracing::{error, info, warn};

use hr_proxy::AppRoute;
use hr_registry::protocol::{AgentMessage, HostRegistryMessage, PowerPolicy, ServiceAction, ServiceType};
use hr_registry::types::{CreateApplicationRequest, TriggerUpdateRequest, UpdateApplicationRequest};
use hr_common::events::{MigrationPhase, MigrationProgressEvent};

use crate::state::{ApiState, MigrationState};

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/", get(list_applications).post(create_application))
        .route("/active-migrations", get(active_migrations))
        .route("/{id}", put(update_application).delete(delete_application))
        .route("/{id}/toggle", post(toggle_application))
        .route("/{id}/token", get(regenerate_token))
        .route("/{id}/services/{service_type}/start", post(start_service))
        .route("/{id}/services/{service_type}/stop", post(stop_service))
        .route("/{id}/power-policy", put(update_power_policy))
        .route("/{id}/update/fix", post(fix_agent_update))
        .route("/{id}/migrate", post(start_migration))
        .route("/{id}/migration-status", get(migration_status))
        .route("/agents/version", get(agent_version))
        .route("/agents/binary", get(agent_binary))
        .route("/agents/update", post(trigger_agent_update))
        .route("/agents/update/status", get(get_update_status))
        .route("/agents/ws", get(agent_ws))
        .route("/{id}/terminal", get(terminal_ws))
}

// ── REST handlers ────────────────────────────────────────────

async fn list_applications(State(state): State<ApiState>) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };
    let apps = registry.list_applications().await;
    Json(serde_json::json!({"success": true, "applications": apps})).into_response()
}

async fn create_application(
    State(state): State<ApiState>,
    Json(req): Json<CreateApplicationRequest>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    match registry.create_application(req).await {
        Ok((app, token)) => {
            info!(slug = app.slug, "Application created via API");
            Json(serde_json::json!({"success": true, "application": app, "token": token})).into_response()
        }
        Err(e) => {
            error!("Failed to create application: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"success": false, "error": e.to_string()}))).into_response()
        }
    }
}

async fn update_application(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateApplicationRequest>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    match registry.update_application(&id, req).await {
        Ok(Some(app)) => Json(serde_json::json!({"success": true, "application": app})).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"success": false, "error": "Not found"}))).into_response(),
        Err(e) => {
            error!("Failed to update application: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"success": false, "error": e.to_string()}))).into_response()
        }
    }
}

async fn delete_application(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    // Remove app routes before deleting
    {
        let apps = registry.list_applications().await;
        if let Some(app) = apps.iter().find(|a| a.id == id) {
            let base_domain = &state.env.base_domain;
            for domain in app.domains(base_domain) {
                state.proxy.remove_app_route(&domain);
            }
        }
    }

    match registry.remove_application(&id).await {
        Ok(true) => Json(serde_json::json!({"success": true})).into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"success": false, "error": "Not found"}))).into_response(),
        Err(e) => {
            error!("Failed to delete application: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"success": false, "error": e.to_string()}))).into_response()
        }
    }
}

async fn toggle_application(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    match registry.toggle_application(&id).await {
        Ok(Some(enabled)) => Json(serde_json::json!({"success": true, "enabled": enabled})).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"success": false, "error": "Not found"}))).into_response(),
        Err(e) => {
            error!("Failed to toggle application: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"success": false, "error": e.to_string()}))).into_response()
        }
    }
}

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

// ── Migration handlers ───────────────────────────────────────

#[derive(serde::Deserialize)]
struct MigrateRequest {
    target_host_id: String,
}

async fn start_migration(
    Path(id): Path<String>,
    State(state): State<ApiState>,
    Json(req): Json<MigrateRequest>,
) -> impl IntoResponse {
    let registry = match &state.registry {
        Some(r) => r.clone(),
        None => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Registry not available"}))).into_response(),
    };

    // Validate app exists and get current host_id
    let app = {
        let apps = registry.list_applications().await;
        apps.iter().find(|a| a.id == id).cloned()
    };

    let app = match app {
        Some(a) => a,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Application not found"}))).into_response(),
    };

    if app.host_id == req.target_host_id {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Application is already on target host"}))).into_response();
    }

    // Check target host is connected (unless target is "local")
    if req.target_host_id != "local" && !registry.is_host_connected(&req.target_host_id).await {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Target host is not connected"}))).into_response();
    }

    // Check no migration already in progress for this app
    {
        let migrations = state.migrations.read().await;
        if migrations.values().any(|m| m.app_id == id && m.error.is_none() && !matches!(m.phase, MigrationPhase::Complete | MigrationPhase::Failed)) {
            return (StatusCode::CONFLICT, Json(serde_json::json!({"error": "Migration already in progress"}))).into_response();
        }
    }

    let transfer_id = uuid::Uuid::new_v4().to_string();
    let migration_state = MigrationState {
        app_id: id.clone(),
        transfer_id: transfer_id.clone(),
        source_host_id: app.host_id.clone(),
        target_host_id: req.target_host_id.clone(),
        phase: MigrationPhase::Stopping,
        progress_pct: 0,
        bytes_transferred: 0,
        total_bytes: 0,
        started_at: chrono::Utc::now(),
        error: None,
    };

    state.migrations.write().await.insert(transfer_id.clone(), migration_state);

    // Spawn migration orchestration
    let migrations = state.migrations.clone();
    let events = state.events.clone();
    let app_id = id.clone();
    let source_host_id = app.host_id.clone();
    let target_host_id = req.target_host_id.clone();
    let slug = app.slug.clone();
    let tid = transfer_id.clone();

    tokio::spawn(async move {
        run_migration(
            registry, migrations, events,
            app_id, slug, tid,
            source_host_id, target_host_id,
        ).await;
    });

    Json(serde_json::json!({"transfer_id": transfer_id, "status": "started"})).into_response()
}

async fn migration_status(
    Path(id): Path<String>,
    State(state): State<ApiState>,
) -> impl IntoResponse {
    let migrations = state.migrations.read().await;

    // Find the most recent migration for this app
    let migration = migrations.values()
        .filter(|m| m.app_id == id)
        .max_by_key(|m| m.started_at);

    match migration {
        Some(m) => Json(serde_json::json!({
            "transfer_id": m.transfer_id,
            "phase": m.phase,
            "progress_pct": m.progress_pct,
            "bytes_transferred": m.bytes_transferred,
            "total_bytes": m.total_bytes,
            "source_host_id": m.source_host_id,
            "target_host_id": m.target_host_id,
            "error": m.error,
        })).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "No migration found"}))).into_response(),
    }
}

async fn active_migrations(
    State(state): State<ApiState>,
) -> impl IntoResponse {
    let migrations = state.migrations.read().await;
    let active: Vec<_> = migrations.values()
        .filter(|m| !matches!(m.phase, MigrationPhase::Complete | MigrationPhase::Failed))
        .collect();
    Json(serde_json::json!({ "migrations": active })).into_response()
}

async fn regenerate_token(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    match registry.regenerate_token(&id).await {
        Ok(Some(token)) => {
            info!(app_id = id, "Token regenerated via API");
            Json(serde_json::json!({"success": true, "token": token})).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"success": false, "error": "Not found"}))).into_response(),
        Err(e) => {
            error!("Failed to regenerate token: {e}");
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

/// Fix a failed agent update via LXC exec (local) or remote exec (remote host).
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
            match registry.fix_agent_via_lxc(&id).await {
                Ok(output) => {
                    info!(app_id = id, "Agent fixed via LXC exec");
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

    // Notify registry of connection (pushes config)
    if let Err(e) = registry.on_agent_connected(&app_id, tx, version, reported_ipv4).await {
        error!(app_id, "Agent provisioning failed: {e}");
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
                                // Forward metrics to registry for storage and broadcast
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

    // Remove app routes for this agent's domains
    {
        let apps = registry.list_applications().await;
        if let Some(app) = apps.iter().find(|a| a.id == app_id) {
            let base_domain = &state.env.base_domain;
            for domain in app.domains(base_domain) {
                state.proxy.remove_app_route(&domain);
            }
        }
    }

    registry.on_agent_disconnected(&app_id).await;
    info!(app_id, "Agent WebSocket closed");
}

// ── Migration orchestration ──────────────────────────────────

// Helper to update migration state and emit event
async fn update_migration_phase(
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

async fn run_migration(
    registry: Arc<hr_registry::AgentRegistry>,
    migrations: Arc<tokio::sync::RwLock<std::collections::HashMap<String, MigrationState>>>,
    events: Arc<hr_common::events::EventBus>,
    app_id: String,
    slug: String,
    transfer_id: String,
    source_host_id: String,
    target_host_id: String,
) {
    let container_name = format!("hr-{}", slug);

    // Phase 1: Stop the agent
    update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Stopping, 0, 0, 0, None).await;

    // Send shutdown to the app's LXC agent
    let _ = registry.send_service_command(
        &app_id,
        ServiceType::App,
        ServiceAction::Stop,
    ).await;

    // Wait a bit for the agent to disconnect
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    // Phase 2: Export
    update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Exporting, 10, 0, 0, None).await;

    if source_host_id == "local" {
        // Local export: use lxc CLI directly
        let stop_result = tokio::process::Command::new("lxc")
            .args(["stop", &container_name, "--force"])
            .output()
            .await;

        if let Err(e) = stop_result {
            update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Failed, 0, 0, 0, Some(format!("Failed to stop container: {}", e))).await;
            return;
        }

        let export_path = format!("/tmp/{}.tar.gz", transfer_id);
        let export_result = tokio::process::Command::new("lxc")
            .args(["export", &container_name, &export_path])
            .output()
            .await;

        match export_result {
            Ok(output) if output.status.success() => {
                // Get file size
                let metadata = match tokio::fs::metadata(&export_path).await {
                    Ok(m) => m,
                    Err(e) => {
                        update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Failed, 0, 0, 0, Some(format!("Failed to read export: {}", e))).await;
                        return;
                    }
                };
                let total_bytes = metadata.len();

                update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Transferring, 20, 0, total_bytes, None).await;

                // If target is remote, send chunks via host-agent
                if target_host_id != "local" {
                    // Register migration signal BEFORE starting transfer
                    let import_rx = registry.register_migration_signal(&transfer_id).await;

                    // Tell target to prepare for import
                    if let Err(e) = registry.send_host_command(
                        &target_host_id,
                        HostRegistryMessage::StartImport {
                            container_name: container_name.clone(),
                            transfer_id: transfer_id.clone(),
                        },
                    ).await {
                        update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Failed, 0, 0, 0, Some(format!("Failed to notify target: {}", e))).await;
                        let _ = tokio::fs::remove_file(&export_path).await;
                        return;
                    }

                    // Stream the file in 64KB chunks
                    use tokio::io::AsyncReadExt;
                    use base64::Engine;
                    let mut file = match tokio::fs::File::open(&export_path).await {
                        Ok(f) => f,
                        Err(e) => {
                            update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Failed, 0, 0, 0, Some(format!("Failed to open export: {}", e))).await;
                            return;
                        }
                    };

                    let mut buf = vec![0u8; 65536];
                    let mut transferred: u64 = 0;
                    let mut chunk_count: u64 = 0;
                    loop {
                        let n = match file.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(e) => {
                                update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Failed, 0, transferred, total_bytes, Some(format!("Read error: {}", e))).await;
                                let _ = tokio::fs::remove_file(&export_path).await;
                                return;
                            }
                        };

                        let encoded = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                        if let Err(e) = registry.send_host_command(
                            &target_host_id,
                            HostRegistryMessage::ReceiveChunk {
                                transfer_id: transfer_id.clone(),
                                data: encoded,
                            },
                        ).await {
                            update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Failed, 0, transferred, total_bytes, Some(format!("Send chunk failed: {}", e))).await;
                            let _ = tokio::fs::remove_file(&export_path).await;
                            return;
                        }

                        transferred += n as u64;
                        chunk_count += 1;
                        let pct = (20 + (transferred * 60 / total_bytes.max(1))) as u8;

                        // Throttle: broadcast every 8 chunks (~512KB) or on last chunk
                        if chunk_count % 8 == 0 || transferred >= total_bytes {
                            update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Transferring, pct.min(80), transferred, total_bytes, None).await;
                        } else {
                            // Update in-memory state only (no broadcast)
                            let mut m = migrations.write().await;
                            if let Some(state) = m.get_mut(&transfer_id) {
                                state.progress_pct = pct.min(80);
                                state.bytes_transferred = transferred;
                            }
                        }
                    }

                    // Tell target transfer is complete
                    let _ = registry.send_host_command(
                        &target_host_id,
                        HostRegistryMessage::TransferComplete {
                            transfer_id: transfer_id.clone(),
                        },
                    ).await;

                    // Clean up local export
                    let _ = tokio::fs::remove_file(&export_path).await;

                    // Phase 4: Importing (target does this)
                    update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Importing, 85, transferred, total_bytes, None).await;

                    // Wait for ImportComplete/ImportFailed from target host-agent
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(120),
                        import_rx,
                    ).await {
                        Ok(Ok(hr_registry::MigrationResult::ImportComplete { .. })) => {
                            info!(transfer_id, "Import confirmed by target host");
                        }
                        Ok(Ok(hr_registry::MigrationResult::ImportFailed { error })) => {
                            update_migration_phase(&migrations, &events, &app_id, &transfer_id,
                                MigrationPhase::Failed, 0, 0, 0,
                                Some(format!("Migration failed on target: {}", error))).await;
                            return;
                        }
                        Ok(Ok(hr_registry::MigrationResult::ExportFailed { error })) => {
                            update_migration_phase(&migrations, &events, &app_id, &transfer_id,
                                MigrationPhase::Failed, 0, 0, 0,
                                Some(format!("Migration failed on target: {}", error))).await;
                            return;
                        }
                        Ok(Err(_)) => {
                            update_migration_phase(&migrations, &events, &app_id, &transfer_id,
                                MigrationPhase::Failed, 0, 0, 0,
                                Some("Migration signal lost".to_string())).await;
                            return;
                        }
                        Err(_) => {
                            update_migration_phase(&migrations, &events, &app_id, &transfer_id,
                                MigrationPhase::Failed, 0, 0, 0,
                                Some("Import timed out after 120s".to_string())).await;
                            return;
                        }
                    }

                } else {
                    // Target is local — import directly
                    update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Importing, 80, total_bytes, total_bytes, None).await;

                    // Pre-create workspace storage volume so import doesn't fail on missing volume reference
                    let vol_name = format!("{container_name}-workspace");
                    let _ = tokio::process::Command::new("lxc")
                        .args(["storage", "volume", "create", "default", &vol_name])
                        .output()
                        .await;

                    let import_result = tokio::process::Command::new("lxc")
                        .args(["import", &export_path])
                        .output()
                        .await;

                    let _ = tokio::fs::remove_file(&export_path).await;

                    match import_result {
                        Ok(output) if output.status.success() => {}
                        Ok(output) => {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Failed, 0, 0, 0, Some(format!("Import failed: {}", stderr))).await;
                            return;
                        }
                        Err(e) => {
                            update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Failed, 0, 0, 0, Some(format!("Import error: {}", e))).await;
                            return;
                        }
                    }
                }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Failed, 0, 0, 0, Some(format!("Export failed: {}", stderr))).await;
                return;
            }
            Err(e) => {
                update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Failed, 0, 0, 0, Some(format!("Export error: {}", e))).await;
                return;
            }
        }
    } else {
        // Source is remote: tell source host-agent to export
        // Register signal to detect export failures
        let export_rx = registry.register_migration_signal(&transfer_id).await;
        // Store transfer_id → container_name mapping so the WS handler can look it up
        registry.set_transfer_container_name(&transfer_id, &container_name).await;

        if let Err(e) = registry.send_host_command(
            &source_host_id,
            HostRegistryMessage::StartExport {
                container_name: container_name.clone(),
                transfer_id: transfer_id.clone(),
            },
        ).await {
            update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Failed, 0, 0, 0, Some(format!("Failed to start export: {}", e))).await;
            return;
        }

        // Wait for the full remote→local pipeline:
        // 1. Remote host-agent exports & streams chunks via WebSocket
        // 2. Master's WS handler receives chunks, writes to /tmp/{transfer_id}.tar.gz
        // 3. On TransferComplete, master runs lxc import + lxc start
        // 4. Master signals ImportComplete on the migration channel
        update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Exporting, 30, 0, 0, None).await;

        match tokio::time::timeout(
            std::time::Duration::from_secs(600),
            export_rx,
        ).await {
            Ok(Ok(hr_registry::MigrationResult::ExportFailed { error })) => {
                update_migration_phase(&migrations, &events, &app_id, &transfer_id,
                    MigrationPhase::Failed, 0, 0, 0,
                    Some(format!("Export failed on source: {}", error))).await;
                return;
            }
            Ok(Ok(hr_registry::MigrationResult::ImportFailed { error })) => {
                update_migration_phase(&migrations, &events, &app_id, &transfer_id,
                    MigrationPhase::Failed, 0, 0, 0,
                    Some(format!("Import failed: {}", error))).await;
                return;
            }
            Ok(Ok(hr_registry::MigrationResult::ImportComplete { .. })) => {
                info!(transfer_id, "Remote migration confirmed");
            }
            Ok(Err(_)) => {
                update_migration_phase(&migrations, &events, &app_id, &transfer_id,
                    MigrationPhase::Failed, 0, 0, 0,
                    Some("Migration signal lost".to_string())).await;
                return;
            }
            Err(_) => {
                update_migration_phase(&migrations, &events, &app_id, &transfer_id,
                    MigrationPhase::Failed, 0, 0, 0,
                    Some("Remote migration timed out after 600s".to_string())).await;
                return;
            }
        }
    }

    // Phase 5: Starting
    update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Starting, 90, 0, 0, None).await;

    // Update application's host_id in registry
    let update_req = UpdateApplicationRequest {
        host_id: Some(target_host_id.clone()),
        ..Default::default()
    };
    if let Err(e) = registry.update_application(&app_id, update_req).await {
        warn!("Failed to update app host_id after migration: {}", e);
    }

    // Delete source container (cleanup)
    if source_host_id == "local" {
        let _ = hr_lxd::LxdClient::delete_container(&container_name).await;
    } else {
        let _ = registry.send_host_command(
            &source_host_id,
            HostRegistryMessage::DeleteContainer {
                container_name: container_name.clone(),
            },
        ).await;
    }

    // Phase 6: Complete
    update_migration_phase(&migrations, &events, &app_id, &transfer_id,MigrationPhase::Complete, 100, 0, 0, None).await;
    info!(app_id, transfer_id, "Migration complete: {} → {}", source_host_id, target_host_id);
}

// ── WebSocket terminal (lxc exec) ───────────────────────────

async fn terminal_ws(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_terminal_ws(state, id, socket))
}

async fn handle_terminal_ws(state: ApiState, app_id: String, mut socket: WebSocket) {
    let Some(registry) = &state.registry else {
        let _ = socket.send(Message::Close(None)).await;
        return;
    };

    // Look up the application to get the container name
    let apps = registry.list_applications().await;
    let Some(app) = apps.iter().find(|a| a.id == app_id) else {
        let _ = socket
            .send(Message::Text(
                serde_json::json!({"error": "Application not found"})
                    .to_string()
                    .into(),
            ))
            .await;
        let _ = socket.send(Message::Close(None)).await;
        return;
    };
    let container = app.container_name.clone();

    info!(container, "Terminal WebSocket opened");

    // Spawn lxc exec with interactive shell
    let mut child = match Command::new("lxc")
        .args([
            "exec",
            &container,
            "--force-interactive",
            "--env",
            "TERM=xterm-256color",
            "--",
            "/bin/bash",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            error!(container, "Failed to spawn lxc exec: {e}");
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({"error": format!("Failed to start shell: {e}")})
                        .to_string()
                        .into(),
                ))
                .await;
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();

    let mut stdout_buf = vec![0u8; 4096];
    let mut stderr_buf = vec![0u8; 4096];

    loop {
        tokio::select! {
            // stdout → WebSocket
            n = stdout.read(&mut stdout_buf) => {
                match n {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if socket.send(Message::Binary(stdout_buf[..n].to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            // stderr → WebSocket
            n = stderr.read(&mut stderr_buf) => {
                match n {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if socket.send(Message::Binary(stderr_buf[..n].to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            // WebSocket → stdin
            ws_msg = socket.recv() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        if stdin.write_all(text.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        if stdin.write_all(&data).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            // Process exited
            status = child.wait() => {
                match status {
                    Ok(s) => info!(container, status = ?s, "Shell process exited"),
                    Err(e) => error!(container, "Shell process error: {e}"),
                }
                break;
            }
        }
    }

    // Clean up
    let _ = child.kill().await;
    let _ = socket.send(Message::Close(None)).await;
    info!(container, "Terminal WebSocket closed");
}
