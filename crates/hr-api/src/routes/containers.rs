//! REST API + WebSocket routes for Containers V2 (systemd-nspawn).

use std::sync::atomic::Ordering;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tracing::{error, info};

use hr_common::events::MigrationPhase;

use crate::container_manager::{
    ContainerV2Config, CreateContainerRequest, MigrateContainerRequest,
};
use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/", get(list_containers).post(create_container))
        .route("/{id}", axum::routing::delete(delete_container))
        .route("/{id}/start", post(start_container))
        .route("/{id}/stop", post(stop_container))
        .route("/{id}/terminal", get(terminal_ws))
        .route("/{id}/migrate", post(migrate_container))
        .route("/{id}/migrate/status", get(migration_status))
        .route("/{id}/migrate/cancel", post(cancel_migration))
        .route("/config", get(get_config).put(update_config))
}

// ── CRUD handlers ────────────────────────────────────────────────

async fn list_containers(State(state): State<ApiState>) -> impl IntoResponse {
    let Some(ref mgr) = state.container_manager else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"success": false, "error": "Container manager not available"})),
        )
            .into_response();
    };
    let containers = mgr.list_containers().await;
    Json(serde_json::json!({"success": true, "containers": containers})).into_response()
}

async fn create_container(
    State(state): State<ApiState>,
    Json(req): Json<CreateContainerRequest>,
) -> impl IntoResponse {
    let Some(ref mgr) = state.container_manager else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"success": false, "error": "Container manager not available"})),
        )
            .into_response();
    };

    match mgr.create_container(req).await {
        Ok((record, token)) => {
            info!(slug = record.slug, "Container V2 created via API");
            Json(serde_json::json!({
                "success": true,
                "container": record,
                "token": token
            }))
            .into_response()
        }
        Err(e) => {
            error!("Failed to create container V2: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"success": false, "error": e})),
            )
                .into_response()
        }
    }
}

async fn delete_container(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(ref mgr) = state.container_manager else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"success": false, "error": "Container manager not available"})),
        )
            .into_response();
    };

    match mgr.remove_container(&id).await {
        Ok(true) => Json(serde_json::json!({"success": true})).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"success": false, "error": "Not found"})),
        )
            .into_response(),
        Err(e) => {
            error!("Failed to delete container V2: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"success": false, "error": e})),
            )
                .into_response()
        }
    }
}

async fn start_container(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(ref mgr) = state.container_manager else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"success": false, "error": "Container manager not available"})),
        )
            .into_response();
    };

    match mgr.start_container(&id).await {
        Ok(true) => Json(serde_json::json!({"success": true})).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"success": false, "error": "Not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"success": false, "error": e})),
        )
            .into_response(),
    }
}

async fn stop_container(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(ref mgr) = state.container_manager else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"success": false, "error": "Container manager not available"})),
        )
            .into_response();
    };

    match mgr.stop_container(&id).await {
        Ok(true) => Json(serde_json::json!({"success": true})).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"success": false, "error": "Not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"success": false, "error": e})),
        )
            .into_response(),
    }
}

// ── Config handlers ──────────────────────────────────────────────

async fn get_config(State(state): State<ApiState>) -> impl IntoResponse {
    let Some(ref mgr) = state.container_manager else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"success": false, "error": "Container manager not available"})),
        )
            .into_response();
    };

    let config = mgr.get_config().await;
    Json(serde_json::json!({"success": true, "config": config})).into_response()
}

async fn update_config(
    State(state): State<ApiState>,
    Json(config): Json<ContainerV2Config>,
) -> impl IntoResponse {
    let Some(ref mgr) = state.container_manager else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"success": false, "error": "Container manager not available"})),
        )
            .into_response();
    };

    match mgr.update_config(config).await {
        Ok(()) => Json(serde_json::json!({"success": true})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"success": false, "error": e})),
        )
            .into_response(),
    }
}

// ── Migration handlers ───────────────────────────────────────────

async fn migrate_container(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<MigrateContainerRequest>,
) -> impl IntoResponse {
    let Some(ref mgr) = state.container_manager else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"success": false, "error": "Container manager not available"})),
        )
            .into_response();
    };

    match mgr
        .migrate_container(&id, &req.target_host_id, &state.migrations)
        .await
    {
        Ok(transfer_id) => {
            Json(serde_json::json!({"transfer_id": transfer_id, "status": "started"}))
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

async fn migration_status(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let migrations = state.migrations.read().await;

    let migration = migrations
        .values()
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
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No migration found"})),
        )
            .into_response(),
    }
}

async fn cancel_migration(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let migrations = state.migrations.read().await;

    let migration = migrations.values().find(|m| {
        m.app_id == id
            && !matches!(
                m.phase,
                MigrationPhase::Complete | MigrationPhase::Failed
            )
    });

    match migration {
        Some(m) => {
            if m.cancelled.load(Ordering::SeqCst) {
                return Json(
                    serde_json::json!({"success": true, "message": "Migration already being cancelled"}),
                )
                .into_response();
            }
            m.cancelled.store(true, Ordering::SeqCst);
            info!(app_id = %id, transfer_id = %m.transfer_id, "Container V2 migration cancel requested");
            Json(
                serde_json::json!({"success": true, "message": "Migration cancellation requested"}),
            )
            .into_response()
        }
        None => {
            let has_any = migrations.values().any(|m| m.app_id == id);
            if has_any {
                Json(
                    serde_json::json!({"success": true, "message": "No active migration to cancel"}),
                )
                .into_response()
            } else {
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "No migration found"})),
                )
                    .into_response()
            }
        }
    }
}

// ── Terminal WebSocket (machinectl shell) ─────────────────────────

async fn terminal_ws(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_terminal_ws(state, id, socket))
}

async fn handle_terminal_ws(state: ApiState, container_id: String, mut socket: WebSocket) {
    let Some(ref mgr) = state.container_manager else {
        let _ = socket.send(Message::Close(None)).await;
        return;
    };

    // Look up the container record to get the container name
    let containers = mgr.list_containers().await;
    let container_name = containers
        .iter()
        .find(|c| c.get("id").and_then(|v| v.as_str()) == Some(&container_id))
        .and_then(|c| c.get("container_name").and_then(|v| v.as_str()))
        .map(|s| s.to_string());

    let Some(container) = container_name else {
        let _ = socket
            .send(Message::Text(
                serde_json::json!({"error": "Container not found"})
                    .to_string()
                    .into(),
            ))
            .await;
        let _ = socket.send(Message::Close(None)).await;
        return;
    };

    info!(container, "Container V2 terminal WebSocket opened");

    // Get the container's leader PID via machinectl show
    let leader_pid = match Command::new("machinectl")
        .args(["show", &container, "--property=Leader", "--value"])
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(container, "Failed to get leader PID: {stderr}");
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({"error": format!("Failed to get container PID: {stderr}")})
                        .to_string()
                        .into(),
                ))
                .await;
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
        Err(e) => {
            error!(container, "Failed to run machinectl show: {e}");
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({"error": format!("Failed to get container PID: {e}")})
                        .to_string()
                        .into(),
                ))
                .await;
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };

    // Use script to allocate a PTY for nsenter+bash (interactive shell with echo/prompts)
    let nsenter_cmd = format!(
        "nsenter -t {} -m -u -i -n -p -- /bin/bash -l",
        leader_pid
    );
    let mut child = match Command::new("script")
        .args(["-qfec", &nsenter_cmd, "/dev/null"])
        .env("TERM", "xterm-256color")
        .env("HOME", "/root")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            error!(container, "Failed to spawn nsenter shell: {e}");
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
            status = child.wait() => {
                match status {
                    Ok(s) => info!(container, status = ?s, "Shell process exited"),
                    Err(e) => error!(container, "Shell process error: {e}"),
                }
                break;
            }
        }
    }

    let _ = child.kill().await;
    let _ = socket.send(Message::Close(None)).await;
    info!(container, "Container V2 terminal WebSocket closed");
}
