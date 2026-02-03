use axum::{
    extract::{State, WebSocketUpgrade, ws::{Message, WebSocket}},
    response::IntoResponse,
    routing::get,
    Router,
};
use serde_json::json;
use tokio::sync::broadcast;
use tracing::{debug, warn};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new().route("/ws", get(ws_handler))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<ApiState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: ApiState) {
    debug!("WebSocket client connected");

    let mut server_rx = state.events.server_status.subscribe();
    let mut updates_rx = state.events.updates.subscribe();
    let mut agent_rx = state.events.agent_status.subscribe();
    let mut metrics_rx = state.events.agent_metrics.subscribe();
    let mut service_cmd_rx = state.events.service_command.subscribe();
    let mut agent_update_rx = state.events.agent_update.subscribe();

    loop {
        tokio::select! {
            // Server status events
            result = server_rx.recv() => {
                match result {
                    Ok(event) => {
                        let msg = json!({
                            "type": "servers:status",
                            "data": {
                                "serverId": event.server_id,
                                "online": event.status == "online",
                                "status": event.status,
                                "latency": event.latency_ms.unwrap_or(0),
                                "lastSeen": chrono::Utc::now().to_rfc3339()
                            }
                        });
                        if socket.send(Message::Text(msg.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WebSocket server_status lagged by {}", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // Update events
            result = updates_rx.recv() => {
                match result {
                    Ok(event) => {
                        use hr_common::events::UpdateEvent;
                        let msg = match event {
                            UpdateEvent::Started => json!({"type": "updates:started"}),
                            UpdateEvent::Phase { phase, message } => json!({"type": "updates:phase", "data": {"phase": phase, "message": message}}),
                            UpdateEvent::Output { line } => json!({"type": "updates:output", "data": {"line": line}}),
                            UpdateEvent::AptComplete { packages, security_count } => json!({"type": "updates:apt-complete", "data": {"packages": packages, "securityCount": security_count}}),
                            UpdateEvent::SnapComplete { snaps } => json!({"type": "updates:snap-complete", "data": {"snaps": snaps}}),
                            UpdateEvent::NeedrestartComplete(data) => json!({"type": "updates:needrestart-complete", "data": data}),
                            UpdateEvent::Complete { success, summary, duration } => json!({"type": "updates:complete", "data": {"success": success, "summary": summary, "duration": duration}}),
                            UpdateEvent::Cancelled => json!({"type": "updates:cancelled"}),
                            UpdateEvent::Error { error } => json!({"type": "updates:error", "data": {"error": error}}),
                            UpdateEvent::UpgradeStarted { upgrade_type } => json!({"type": "updates:upgrade-started", "data": {"type": upgrade_type}}),
                            UpdateEvent::UpgradeOutput { line } => json!({"type": "updates:upgrade-output", "data": {"line": line}}),
                            UpdateEvent::UpgradeComplete { upgrade_type, success, duration, error } => json!({"type": "updates:upgrade-complete", "data": {"type": upgrade_type, "success": success, "duration": duration, "error": error}}),
                            UpdateEvent::UpgradeCancelled => json!({"type": "updates:upgrade-cancelled"}),
                        };
                        if socket.send(Message::Text(msg.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WebSocket updates lagged by {}", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // Agent status events
            result = agent_rx.recv() => {
                match result {
                    Ok(event) => {
                        let mut data = json!({
                            "appId": event.app_id,
                            "slug": event.slug,
                            "status": event.status
                        });
                        if let Some(message) = &event.message {
                            data["message"] = json!(message);
                        }
                        let msg = json!({
                            "type": "agent:status",
                            "data": data
                        });
                        if socket.send(Message::Text(msg.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WebSocket agent_status lagged by {}", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // Agent metrics events
            result = metrics_rx.recv() => {
                match result {
                    Ok(event) => {
                        let msg = json!({
                            "type": "agent:metrics",
                            "data": {
                                "appId": event.app_id,
                                "codeServerStatus": event.code_server_status,
                                "appStatus": event.app_status,
                                "dbStatus": event.db_status,
                                "memoryBytes": event.memory_bytes,
                                "cpuPercent": event.cpu_percent,
                                "codeServerIdleSecs": event.code_server_idle_secs,
                                "appIdleSecs": event.app_idle_secs,
                            }
                        });
                        if socket.send(Message::Text(msg.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WebSocket agent_metrics lagged by {}", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // Service command completion events
            result = service_cmd_rx.recv() => {
                match result {
                    Ok(event) => {
                        let msg = json!({
                            "type": "agent:service-command",
                            "data": {
                                "appId": event.app_id,
                                "serviceType": event.service_type,
                                "action": event.action,
                                "success": event.success,
                            }
                        });
                        if socket.send(Message::Text(msg.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WebSocket service_command lagged by {}", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // Agent update events
            result = agent_update_rx.recv() => {
                match result {
                    Ok(event) => {
                        let msg = json!({
                            "type": "agent:update",
                            "data": {
                                "appId": event.app_id,
                                "slug": event.slug,
                                "status": format!("{:?}", event.status).to_lowercase(),
                                "version": event.version,
                                "error": event.error,
                            }
                        });
                        if socket.send(Message::Text(msg.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WebSocket agent_update lagged by {}", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // Client disconnect
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    _ => {} // Ignore other messages
                }
            }
        }
    }

    debug!("WebSocket client disconnected");
}
