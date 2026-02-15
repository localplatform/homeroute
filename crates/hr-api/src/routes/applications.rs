//! REST API + WebSocket routes for application management.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::extract::DefaultBodyLimit;
use axum::{Json, Router};
use tokio::io::AsyncReadExt;
use tracing::{error, info, warn};

use hr_proxy::AppRoute;
use hr_registry::protocol::{AgentMessage, HostRegistryMessage, PowerPolicy, ServiceAction, ServiceConfig, ServiceType};
use hr_registry::types::{TriggerUpdateRequest, UpdateApplicationRequest};
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
        .route("/{id}/exec", post(exec_in_container))
        .route("/{id}/deploy", post(deploy_to_production).layer(DefaultBodyLimit::max(200 * 1024 * 1024)))
        .route("/{id}/prod/status", get(get_prod_status))
        .route("/{id}/prod/logs", get(get_prod_logs))
        .route("/{id}/prod/exec", post(prod_exec))
        .route("/{id}/prod/push", post(prod_push).layer(DefaultBodyLimit::max(200 * 1024 * 1024)))
        .route("/deploys/{deploy_id}/artifact", get(get_deploy_artifact))
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

// ── Deploy (dev → prod) handlers ─────────────────────────────

/// POST /api/applications/{dev_id}/deploy
/// Accepts raw binary body (application/octet-stream).
/// Copies binary to /opt/app/app in prod, creates systemd unit if needed, restarts service.
/// Synchronous — blocks until deploy completes.
async fn deploy_to_production(
    State(state): State<ApiState>,
    Path(dev_id): Path<String>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    if body.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"success": false, "error": "Empty body — send the binary as raw bytes"}))).into_response();
    }

    // Look up the dev app
    let dev_app = match registry.get_application(&dev_id).await {
        Some(app) => app,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"success": false, "error": "Dev application not found"}))).into_response(),
    };

    // Validate it's a dev container
    if dev_app.environment != hr_registry::types::Environment::Development {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"success": false, "error": "Source application is not a development environment"}))).into_response();
    }

    // Look up linked prod container
    let prod_id = match &dev_app.linked_app_id {
        Some(id) => id.clone(),
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"success": false, "error": "No linked production application"}))).into_response(),
    };

    let prod_app = match registry.get_application(&prod_id).await {
        Some(app) => app,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"success": false, "error": "Linked production application not found"}))).into_response(),
    };

    if prod_app.environment != hr_registry::types::Environment::Production {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"success": false, "error": "Linked application is not a production environment"}))).into_response();
    }

    let binary_size = body.len();
    info!(dev_id, prod_id = prod_id.as_str(), binary_bytes = binary_size, "Deploy binary to production");

    // Execute deploy synchronously
    match execute_deploy(
        registry,
        &prod_id,
        &prod_app.container_name,
        &prod_app.host_id,
        body.to_vec(),
    ).await {
        Ok(msg) => Json(serde_json::json!({
            "success": true,
            "message": msg,
            "prod_id": prod_id,
            "binary_size": binary_size,
        })).into_response(),
        Err(err) => {
            error!(dev_id, prod_id = prod_id.as_str(), "Deploy failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                "success": false,
                "error": err,
            }))).into_response()
        }
    }
}

/// Execute the deploy pipeline synchronously: setup → stop → copy binary → start.
/// Returns Ok(message) on success or Err(error) on failure.
async fn execute_deploy(
    registry: &Arc<hr_registry::AgentRegistry>,
    prod_id: &str,
    prod_container: &str,
    prod_host: &str,
    binary_data: Vec<u8>,
) -> Result<String, String> {
    let deploy_id = uuid::Uuid::new_v4().to_string();

    // Phase 0: Ensure /opt/app exists and app.service runs /opt/app/app directly
    let setup_cmd = r#"mkdir -p /opt/app && if [ ! -f /etc/systemd/system/app.service ]; then cat > /etc/systemd/system/app.service << 'SVCEOF'
[Unit]
Description=Application Service
After=network.target

[Service]
Type=simple
WorkingDirectory=/opt/app
ExecStart=/opt/app/app
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
SVCEOF
systemctl daemon-reload
systemctl enable app.service
fi"#;

    let setup_result = if prod_host == "local" {
        let out = tokio::process::Command::new("machinectl")
            .args(["shell", prod_container, "/bin/bash", "-c", setup_cmd])
            .output()
            .await;
        out.map(|o| o.status.success()).unwrap_or(false)
    } else {
        registry.exec_in_remote_container(prod_host, prod_container, vec![setup_cmd.to_string()])
            .await.map(|(ok, _, _)| ok).unwrap_or(false)
    };
    if !setup_result {
        warn!(deploy_id, "Setup command returned non-zero (may already be configured)");
    }

    // Ensure prod app's ServiceConfig includes app.service
    let update_req = UpdateApplicationRequest {
        services: Some(ServiceConfig {
            app: vec!["app.service".to_string()],
            db: vec![],
        }),
        ..Default::default()
    };
    if let Err(e) = registry.update_application(prod_id, update_req).await {
        warn!(deploy_id, "Failed to update prod ServiceConfig: {e}");
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Phase 1: Stop prod app service
    info!(deploy_id, "Deploy: stopping prod app service");
    match registry.send_service_command(prod_id, ServiceType::App, ServiceAction::Stop).await {
        Ok(true) => {}
        Ok(false) => warn!(deploy_id, "Prod agent not connected, skipping service stop"),
        Err(e) => warn!(deploy_id, "Failed to stop prod service (continuing): {e}"),
    }
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Phase 2: Copy binary to /opt/app/app in prod container
    info!(deploy_id, binary_bytes = binary_data.len(), "Deploy: copying binary to prod container");

    let tmp_path = format!("/tmp/deploy-{}.bin", deploy_id);
    tokio::fs::write(&tmp_path, &binary_data).await
        .map_err(|e| format!("Failed to write temp binary: {e}"))?;

    if prod_host == "local" {
        // Local: remove old binary first (machinectl copy-to fails if file exists), then copy + chmod
        let _ = tokio::process::Command::new("machinectl")
            .args(["shell", prod_container, "/bin/rm", "-f", "/opt/app/app"])
            .output()
            .await;

        let copy = tokio::process::Command::new("machinectl")
            .args(["copy-to", prod_container, &tmp_path, "/opt/app/app"])
            .output()
            .await;
        match copy {
            Ok(out) if out.status.success() => {}
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(format!("machinectl copy-to failed: {stderr}"));
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(format!("Failed to run machinectl: {e}"));
            }
        }

        let chmod = tokio::process::Command::new("machinectl")
            .args(["shell", prod_container, "/bin/chmod", "+x", "/opt/app/app"])
            .output()
            .await;
        if let Ok(out) = chmod {
            if !out.status.success() {
                warn!(deploy_id, "chmod +x failed (continuing)");
            }
        }
    } else {
        // Remote: container downloads binary via curl from artifact endpoint, then chmod
        let api_port = 4000;
        let download_url = format!(
            "http://10.0.0.254:{}/api/applications/deploys/{}/artifact",
            api_port, deploy_id
        );
        let cmd = vec![format!(
            "curl -fsSL '{}' -o /opt/app/app && chmod +x /opt/app/app",
            download_url
        )];
        match registry.exec_in_remote_container(prod_host, prod_container, cmd).await {
            Ok((true, _, _)) => {
                info!(deploy_id, "Binary downloaded to remote prod container");
            }
            Ok((false, _, stderr)) => {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(format!("Remote deploy failed: {stderr}"));
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(format!("Remote exec failed: {e}"));
            }
        }
    }

    let _ = tokio::fs::remove_file(&tmp_path).await;

    // Phase 3: Start prod app service
    info!(deploy_id, "Deploy: starting prod app service");
    match registry.send_service_command(prod_id, ServiceType::App, ServiceAction::Start).await {
        Ok(true) => {}
        Ok(false) => warn!(deploy_id, "Prod agent not connected, could not start service"),
        Err(e) => warn!(deploy_id, "Failed to start prod service: {e}"),
    }

    info!(deploy_id, "Deploy to production completed successfully");
    Ok(format!("Binary deployed to /opt/app/app and app.service restarted"))
}

/// GET /api/applications/deploys/{deploy_id}/artifact
/// Serves the temporary deploy binary file (used by remote containers to download).
async fn get_deploy_artifact(
    Path(deploy_id): Path<String>,
) -> impl IntoResponse {
    let tmp_path = format!("/tmp/deploy-{}.bin", deploy_id);
    match tokio::fs::read(&tmp_path).await {
        Ok(data) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            data,
        ).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Artifact not found or expired").into_response(),
    }
}

// ── Prod status/logs handlers (queried from dev container) ───

/// Helper: resolve a dev app to its linked prod app and container info.
async fn resolve_linked_prod(
    registry: &Arc<hr_registry::AgentRegistry>,
    dev_id: &str,
) -> Result<(String, String, String), (StatusCode, Json<serde_json::Value>)> {
    let dev_app = registry.get_application(dev_id).await
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(serde_json::json!({"success": false, "error": "Application not found"}))))?;

    let prod_id = dev_app.linked_app_id.as_ref()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({"success": false, "error": "No linked production application"}))))?
        .clone();

    let prod_app = registry.get_application(&prod_id).await
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(serde_json::json!({"success": false, "error": "Linked production application not found"}))))?;

    Ok((prod_id, prod_app.container_name.clone(), prod_app.host_id.clone()))
}

/// Execute a command in a container (local or remote) and return (success, stdout, stderr).
async fn exec_in(
    registry: &Arc<hr_registry::AgentRegistry>,
    container: &str,
    host: &str,
    cmd: &str,
) -> Result<(bool, String, String), String> {
    if host == "local" {
        let out = tokio::process::Command::new("machinectl")
            .args(["shell", container, "/bin/bash", "-c", cmd])
            .output()
            .await
            .map_err(|e| e.to_string())?;
        Ok((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        ))
    } else {
        registry.exec_in_remote_container(host, container, vec![cmd.to_string()])
            .await
            .map_err(|e| e.to_string())
    }
}

/// GET /api/applications/{dev_id}/prod/status
async fn get_prod_status(
    State(state): State<ApiState>,
    Path(dev_id): Path<String>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    let (_, prod_container, prod_host) = match resolve_linked_prod(registry, &dev_id).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    let cmd = r#"echo -n "SERVICE_ACTIVE="; systemctl is-active app.service 2>/dev/null || true; echo -n "SERVICE_STATUS="; systemctl show app.service --property=ActiveState,SubState,MainPID,ExecMainStartTimestamp --no-pager 2>/dev/null || true; echo -n "BINARY_INFO="; stat --printf='%s %Y' /opt/app/app 2>/dev/null || echo "not_found""#;

    match exec_in(registry, &prod_container, &prod_host, cmd).await {
        Ok((_, stdout, _)) => {
            Json(serde_json::json!({
                "success": true,
                "raw": stdout.trim(),
            })).into_response()
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                "success": false,
                "error": format!("Failed to query prod status: {e}"),
            }))).into_response()
        }
    }
}

/// GET /api/applications/{dev_id}/prod/logs?lines=N
async fn get_prod_logs(
    State(state): State<ApiState>,
    Path(dev_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    let (_, prod_container, prod_host) = match resolve_linked_prod(registry, &dev_id).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    let lines = params.get("lines")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(50)
        .min(1000);

    let cmd = format!("journalctl -u app.service -n {} --no-pager 2>&1", lines);

    match exec_in(registry, &prod_container, &prod_host, &cmd).await {
        Ok((_, stdout, _)) => {
            Json(serde_json::json!({
                "success": true,
                "logs": stdout,
            })).into_response()
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                "success": false,
                "error": format!("Failed to query prod logs: {e}"),
            }))).into_response()
        }
    }
}

/// POST /api/applications/{dev_id}/prod/exec
/// Execute a shell command in the linked production container.
/// Body: {"command": "..."}
async fn prod_exec(
    State(state): State<ApiState>,
    Path(dev_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    let command = match body.get("command").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"success": false, "error": "command (string) required"}))).into_response(),
    };

    let (_, prod_container, prod_host) = match resolve_linked_prod(registry, &dev_id).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    match exec_in(registry, &prod_container, &prod_host, command).await {
        Ok((success, stdout, stderr)) => {
            let status = if success { StatusCode::OK } else { StatusCode::OK };
            (status, Json(serde_json::json!({
                "success": success,
                "stdout": stdout,
                "stderr": stderr,
            }))).into_response()
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                "success": false,
                "error": format!("Failed to execute command: {e}"),
            }))).into_response()
        }
    }
}

/// POST /api/applications/{dev_id}/prod/push
/// Push a file or directory to the linked production container.
/// Body: raw tar archive bytes.
/// Headers:
///   X-Remote-Path: destination path on prod (required)
///   X-Is-Directory: "true" if archive should be extracted, "false" for single file
async fn prod_push(
    State(state): State<ApiState>,
    Path(dev_id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    let remote_path = match headers.get("X-Remote-Path").and_then(|v| v.to_str().ok()) {
        Some(p) => p.to_string(),
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"success": false, "error": "X-Remote-Path header required"}))).into_response(),
    };

    let is_directory = headers.get("X-Is-Directory")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "true")
        .unwrap_or(false);

    if body.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"success": false, "error": "Empty body"}))).into_response();
    }

    let (_, prod_container, prod_host) = match resolve_linked_prod(registry, &dev_id).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    let tmp_id = uuid::Uuid::new_v4();
    let tmp_path = format!("/tmp/push-{}.tar", tmp_id);
    if let Err(e) = tokio::fs::write(&tmp_path, &body).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"success": false, "error": format!("Failed to write temp file: {e}")}))).into_response();
    }

    let result = if prod_host == "local" {
        if is_directory {
            // Create remote directory, copy tar, extract, remove tar
            let setup = format!("mkdir -p '{}'", remote_path);
            let _ = exec_in(registry, &prod_container, &prod_host, &setup).await;

            let tar_dest = format!("/tmp/push-{}.tar", tmp_id);

            // Copy tar into container
            let copy = tokio::process::Command::new("machinectl")
                .args(["copy-to", &prod_container, &tmp_path, &tar_dest])
                .output()
                .await;
            match copy {
                Ok(out) if out.status.success() => {
                    // Extract and clean up
                    let extract_cmd = format!("tar xf '{}' -C '{}' && rm -f '{}'", tar_dest, remote_path, tar_dest);
                    exec_in(registry, &prod_container, &prod_host, &extract_cmd).await
                }
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    Err(format!("machinectl copy-to failed: {stderr}"))
                }
                Err(e) => Err(format!("Failed to run machinectl: {e}")),
            }
        } else {
            // Single file: remove old, copy directly
            let rm_cmd = format!("rm -f '{}'", remote_path);
            let _ = exec_in(registry, &prod_container, &prod_host, &rm_cmd).await;

            // Ensure parent directory exists
            if let Some(parent) = std::path::Path::new(&remote_path).parent() {
                let mkdir_cmd = format!("mkdir -p '{}'", parent.display());
                let _ = exec_in(registry, &prod_container, &prod_host, &mkdir_cmd).await;
            }

            // The tar contains a single file — extract it to get the original file,
            // then copy that to the destination
            let extract_dir = format!("/tmp/push-extract-{}", tmp_id);
            std::fs::create_dir_all(&extract_dir).ok();
            let extract_local = tokio::process::Command::new("tar")
                .args(["xf", &tmp_path, "-C", &extract_dir])
                .output()
                .await;
            match extract_local {
                Ok(out) if out.status.success() => {
                    // Find the extracted file
                    let entries: Vec<_> = std::fs::read_dir(&extract_dir)
                        .map(|rd| rd.filter_map(|e| e.ok()).collect())
                        .unwrap_or_default();
                    if let Some(entry) = entries.first() {
                        let local_file = entry.path();
                        let copy = tokio::process::Command::new("machinectl")
                            .args(["copy-to", &prod_container, &local_file.to_string_lossy().as_ref(), &remote_path])
                            .output()
                            .await;
                        let _ = tokio::fs::remove_dir_all(&extract_dir).await;
                        match copy {
                            Ok(out) if out.status.success() => Ok((true, String::new(), String::new())),
                            Ok(out) => {
                                let stderr = String::from_utf8_lossy(&out.stderr);
                                Err(format!("machinectl copy-to failed: {stderr}"))
                            }
                            Err(e) => Err(format!("Failed to run machinectl: {e}")),
                        }
                    } else {
                        let _ = tokio::fs::remove_dir_all(&extract_dir).await;
                        Err("No file found in tar archive".to_string())
                    }
                }
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    Err(format!("Failed to extract tar: {stderr}"))
                }
                Err(e) => Err(format!("Failed to run tar: {e}")),
            }
        }
    } else {
        // Remote host: copy tar to container via remote exec, then extract
        // For remote, we need to make the tar available via HTTP download
        // Reuse the artifact pattern
        let artifact_path = format!("/tmp/push-artifact-{}.tar", tmp_id);
        if let Err(e) = tokio::fs::copy(&tmp_path, &artifact_path).await {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"success": false, "error": format!("Failed to stage artifact: {e}")}))).into_response();
        }

        let download_url = format!(
            "http://10.0.0.254:4000/api/applications/deploys/{}/artifact",
            format!("push-artifact-{}", tmp_id)
        );

        let cmd = if is_directory {
            format!(
                "mkdir -p '{}' && curl -fsSL '{}' | tar x -C '{}'",
                remote_path, download_url, remote_path
            )
        } else {
            // For single file on remote: download tar, extract, move file
            let parent = std::path::Path::new(&remote_path)
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "/".to_string());
            format!(
                "mkdir -p '{}' && curl -fsSL '{}' | tar x -C /tmp/ && mv /tmp/$(tar tf /dev/stdin < /dev/null 2>/dev/null || echo 'extracted_file') '{}'",
                parent, download_url, remote_path
            )
        };

        let res = registry.exec_in_remote_container(&prod_host, &prod_container, vec![cmd]).await
            .map_err(|e| e.to_string());

        let _ = tokio::fs::remove_file(&artifact_path).await;
        res
    };

    let _ = tokio::fs::remove_file(&tmp_path).await;

    match result {
        Ok((true, stdout, _)) => {
            info!(dev_id, remote_path, is_directory, bytes = body.len(), "Pushed to prod container");
            Json(serde_json::json!({
                "success": true,
                "message": format!("Copied to {}", remote_path),
                "bytes": body.len(),
                "stdout": stdout,
            })).into_response()
        }
        Ok((false, stdout, stderr)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                "success": false,
                "error": format!("Command failed: {}", stderr),
                "stdout": stdout,
            }))).into_response()
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                "success": false,
                "error": e,
            }))).into_response()
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

/// POST /api/applications/{id}/exec — execute a command in the container (local or remote).
async fn exec_in_container(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"success": false, "error": "Registry not available"}))).into_response();
    };

    let command: Vec<String> = match body.get("command").and_then(|v| v.as_array()) {
        Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        None => {
            return (StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"success": false, "error": "command (string array) required"}))).into_response();
        }
    };

    let Some(app) = registry.get_application(&id).await else {
        return (StatusCode::NOT_FOUND,
            Json(serde_json::json!({"success": false, "error": "Application not found"}))).into_response();
    };

    let result = if app.host_id == "local" {
        let joined = command.join(" ");
        let output = tokio::process::Command::new("machinectl")
            .args(["shell", &app.container_name, "/bin/bash", "-c", &joined])
            .output()
            .await;
        match output {
            Ok(out) => Ok((out.status.success(),
                String::from_utf8_lossy(&out.stdout).to_string(),
                String::from_utf8_lossy(&out.stderr).to_string())),
            Err(e) => Err(e.to_string()),
        }
    } else {
        registry.exec_in_remote_container(&app.host_id, &app.container_name, command).await
            .map_err(|e| e.to_string())
    };

    match result {
        Ok((true, stdout, _)) => {
            Json(serde_json::json!({"success": true, "stdout": stdout})).into_response()
        }
        Ok((false, stdout, stderr)) => {
            (StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"success": false, "stdout": stdout, "stderr": stderr}))).into_response()
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"success": false, "error": e}))).into_response()
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

/// Add local DNS A records for an agent, based on environment:
/// - Development: `*.{slug}.{base}` → IPv4 (covers dev.{slug} and code.{slug})
/// - Production: `{slug}.{base}` → IPv4
async fn add_agent_dns_records(
    dns: &hr_dns::SharedDnsState,
    slug: &str,
    base_domain: &str,
    ipv4: &str,
    environment: hr_registry::types::Environment,
) {
    let mut dns_state = dns.write().await;
    match environment {
        hr_registry::types::Environment::Development => {
            // Wildcard covers dev.{slug}.{base} and code.{slug}.{base}
            dns_state.add_static_record(StaticRecord {
                name: format!("*.{}.{}", slug, base_domain),
                record_type: "A".to_string(),
                value: ipv4.to_string(),
                ttl: 60,
            });
        }
        hr_registry::types::Environment::Production => {
            // Bare domain for prod
            dns_state.add_static_record(StaticRecord {
                name: format!("{}.{}", slug, base_domain),
                record_type: "A".to_string(),
                value: ipv4.to_string(),
                ttl: 60,
            });
        }
    }
    info!(slug, ipv4, ?environment, "Added local DNS A records for agent");
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
            app_id: None,
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
        app_id: Some(app_id.clone()),
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
                                let apps = registry.list_applications().await;
                                let app_info = apps.iter().find(|a| a.id == app_id);
                                let slug = app_info.map(|a| a.slug.clone()).unwrap_or_default();
                                let app_env = app_info.map(|a| a.environment).unwrap_or_default();
                                let cached = crate::state::CachedDataverseSchema {
                                    app_id: app_id.clone(),
                                    slug,
                                    environment: app_env,
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
                                    add_agent_dns_records(&state.dns, &app.slug, &state.env.base_domain, &ipv4_address, app.environment).await;
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
                                                local_only: app.frontend.local_only,
                                            });
                                        }
                                        // Add local DNS A records for direct local access
                                        let ip_str = target_ip.to_string();
                                        add_agent_dns_records(&state.dns, &app.slug, base_domain, &ip_str, app.environment).await;
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
