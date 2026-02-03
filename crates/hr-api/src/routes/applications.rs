//! REST API + WebSocket routes for application management.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tracing::{debug, error, info, warn};

use hr_registry::protocol::{AgentMessage, PowerPolicy, ServiceAction, ServiceType};
use hr_registry::types::{CreateApplicationRequest, UpdateApplicationRequest};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/", get(list_applications).post(create_application))
        .route("/{id}", put(update_application).delete(delete_application))
        .route("/{id}/toggle", post(toggle_application))
        .route("/{id}/token", get(regenerate_token))
        .route("/{id}/services/{service_type}/start", post(start_service))
        .route("/{id}/services/{service_type}/stop", post(stop_service))
        .route("/{id}/power-policy", put(update_power_policy))
        .route("/agents/version", get(agent_version))
        .route("/agents/binary", get(agent_binary))
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

    // Remove passthrough entries before deleting
    {
        let apps = registry.list_applications().await;
        if let Some(app) = apps.iter().find(|a| a.id == id) {
            let base_domain = &state.env.base_domain;
            for domain in app.domains(base_domain) {
                state.proxy.remove_passthrough(&domain);
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

    let (token, service_name, version, reported_ipv6) = match auth_msg {
        Ok(Some(Ok(Message::Text(text)))) => {
            match serde_json::from_str::<AgentMessage>(&text) {
                Ok(AgentMessage::Auth { token, service_name, version, ipv6_address }) => {
                    (token, service_name, version, ipv6_address)
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

    info!(app_id = app_id, service = service_name, ipv6 = ?reported_ipv6, "Agent authenticated");

    // Create mpsc channel for registry → agent messages
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);

    // Notify registry of connection (provisions certs, DNS, firewall, pushes config)
    if let Err(e) = registry.on_agent_connected(&app_id, tx, version, reported_ipv6).await {
        error!(app_id, "Agent provisioning failed: {e}");
        let _ = socket.send(Message::Close(None)).await;
        return;
    }

    // Update TLS passthrough map so the host proxy forwards agent domains
    {
        let apps = registry.list_applications().await;
        if let Some(app) = apps.iter().find(|a| a.id == app_id) {
            if let Some(ipv6) = app.ipv6_address {
                let base_domain = &state.env.base_domain;
                let target = format!("[{}]:443", ipv6);
                for domain in app.domains(base_domain) {
                    state.proxy.set_passthrough(domain, target.clone());
                }
            }
        }
    }

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

    // Remove TLS passthrough entries for this agent's domains
    {
        let apps = registry.list_applications().await;
        if let Some(app) = apps.iter().find(|a| a.id == app_id) {
            let base_domain = &state.env.base_domain;
            for domain in app.domains(base_domain) {
                state.proxy.remove_passthrough(&domain);
            }
        }
    }

    registry.on_agent_disconnected(&app_id).await;
    info!(app_id, "Agent WebSocket closed");
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
