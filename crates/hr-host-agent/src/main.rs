use futures_util::{SinkExt, StreamExt};
use hr_registry::protocol::{AutoOffMode, HostAgentMessage, HostMetrics, HostRegistryMessage};
use std::collections::HashMap;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

/// Outgoing WebSocket message: either a JSON text message or raw binary data.
enum OutgoingWsMessage {
    Text(HostAgentMessage),
    Binary(Vec<u8>),
}

mod config;
use config::Config;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,hr_host_agent=debug".parse().unwrap()),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/etc/hr-host-agent/config.toml".to_string());

    let config = match Config::load(&std::path::PathBuf::from(&config_path)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Configuration error: {}", e);
            std::process::exit(1);
        }
    };

    info!(
        host = config.host_name,
        target = config.homeroute_url,
        "hr-host-agent starting"
    );

    let mut backoff = config.reconnect_interval_secs;

    loop {
        match run_connection(&config).await {
            Ok(()) => {
                info!("Connection closed normally");
                backoff = config.reconnect_interval_secs;
            }
            Err(e) => {
                error!("Connection error: {}", e);
                backoff = (backoff * 2).min(60);
            }
        }
        info!(secs = backoff, "Reconnecting...");
        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
    }
}

async fn run_connection(config: &Config) -> Result<(), String> {
    let url = config.ws_url();
    info!(url, "Connecting to HomeRoute");

    let (ws_stream, _) = connect_async(&url)
        .await
        .map_err(|e| format!("WebSocket connect failed: {}", e))?;

    let (mut write, mut read) = ws_stream.split();

    // Send Auth
    let auth = HostAgentMessage::Auth {
        token: config.token.clone(),
        host_name: config.host_name.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let auth_json = serde_json::to_string(&auth).map_err(|e| e.to_string())?;
    write
        .send(Message::Text(auth_json.into()))
        .await
        .map_err(|e| e.to_string())?;

    // Wait for AuthResult
    let auth_response = tokio::time::timeout(std::time::Duration::from_secs(5), read.next())
        .await
        .map_err(|_| "Auth timeout".to_string())?
        .ok_or("Connection closed during auth")?
        .map_err(|e| format!("WebSocket error: {}", e))?;

    match auth_response {
        Message::Text(text) => {
            let msg: HostRegistryMessage =
                serde_json::from_str(&text).map_err(|e| format!("Parse auth response: {}", e))?;
            match msg {
                HostRegistryMessage::AuthResult {
                    success: true, ..
                } => {
                    info!("Authenticated successfully");
                }
                HostRegistryMessage::AuthResult {
                    success: false,
                    error,
                } => {
                    return Err(format!("Auth failed: {}", error.unwrap_or_default()));
                }
                _ => return Err("Unexpected auth response".to_string()),
            }
        }
        _ => return Err("Unexpected message type during auth".to_string()),
    }

    // Channel for outgoing messages (Text JSON or Binary frames)
    let (tx, mut rx) = tokio::sync::mpsc::channel::<OutgoingWsMessage>(512);

    // Import phase state machine
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum ImportPhase {
        ReceivingContainer,
        ReceivingWorkspace,
    }

    // Track active nspawn imports
    struct ActiveNspawnImport {
        container_name: String,
        storage_path: String,
        tar_child: tokio::process::Child,
        tar_stdin: tokio::process::ChildStdin,
        phase: ImportPhase,
        ws_tar_child: Option<tokio::process::Child>,
        ws_tar_stdin: Option<tokio::process::ChildStdin>,
        network_mode: String,
    }
    let mut active_nspawn_imports: HashMap<String, ActiveNspawnImport> = HashMap::new();

    // Read nspawn storage path from config
    let _nspawn_storage_path = config.container_storage_path.clone()
        .unwrap_or_else(|| "/var/lib/machines".to_string());

    // Pending binary chunk metadata (from ReceiveChunkBinary, awaiting next Binary frame)
    let mut pending_binary_chunk: Option<(String, u32)> = None; // (transfer_id, checksum)

    // Auto-off: idle monitoring (sleep or shutdown)
    let mut auto_off_mode: Option<AutoOffMode> = None;
    let mut auto_off_minutes: u32 = 0;
    let mut idle_since: Option<tokio::time::Instant> = None;
    const CPU_IDLE_THRESHOLD: f32 = 5.0;

    let (cpu_tx, mut cpu_rx) = tokio::sync::watch::channel(0.0f32);

    // Heartbeat task
    let tx_hb = tx.clone();
    let heartbeat_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let uptime = {
                std::fs::read_to_string("/proc/uptime")
                    .ok()
                    .and_then(|s| {
                        s.split_whitespace()
                            .next()
                            .and_then(|v| v.parse::<f64>().ok())
                    })
                    .unwrap_or(0.0) as u64
            };
            if tx_hb
                .send(OutgoingWsMessage::Text(HostAgentMessage::Heartbeat {
                    uptime_secs: uptime,
                    containers_running: 0,
                }))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // Metrics task (every 5 seconds)
    let tx_metrics = tx.clone();
    let metrics_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let metrics = collect_metrics();
            let cpu = metrics.cpu_percent;
            if tx_metrics
                .send(OutgoingWsMessage::Text(HostAgentMessage::Metrics(metrics)))
                .await
                .is_err()
            {
                break;
            }
            let _ = cpu_tx.send(cpu);
        }
    });

    // Interfaces task - report network interfaces periodically
    let tx_ifaces = tx.clone();
    let ifaces_handle = tokio::spawn(async move {
        // Send once immediately
        let ifaces = collect_interfaces();
        let _ = tx_ifaces.send(OutgoingWsMessage::Text(HostAgentMessage::NetworkInterfaces(ifaces))).await;
        // Then every 5 minutes
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        interval.tick().await; // skip first tick (already sent)
        loop {
            interval.tick().await;
            let ifaces = collect_interfaces();
            if tx_ifaces.send(OutgoingWsMessage::Text(HostAgentMessage::NetworkInterfaces(ifaces))).await.is_err() {
                break;
            }
        }
    });

    // Message loop
    loop {
        tokio::select! {
            // Outgoing messages
            Some(msg) = rx.recv() => {
                match msg {
                    OutgoingWsMessage::Text(agent_msg) => {
                        let text = match serde_json::to_string(&agent_msg) {
                            Ok(t) => t,
                            Err(_) => continue,
                        };
                        if write.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    OutgoingWsMessage::Binary(data) => {
                        if write.send(Message::Binary(data.into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            // Incoming messages
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<HostRegistryMessage>(&text) {
                            Ok(HostRegistryMessage::Shutdown { drain }) => {
                                info!(drain, "Shutdown requested");
                                break;
                            }
                            Ok(HostRegistryMessage::ReceiveChunkBinary { transfer_id, sequence: _, size: _, checksum }) => {
                                // Store metadata; the next Binary frame carries the actual data
                                pending_binary_chunk = Some((transfer_id, checksum));
                            }
                            Ok(HostRegistryMessage::WorkspaceReady { transfer_id, size_bytes }) => {
                                info!(transfer_id = %transfer_id, size_bytes, "Workspace data incoming");

                                if let Some(import) = active_nspawn_imports.get_mut(&transfer_id) {
                                    use tokio::io::AsyncWriteExt;

                                    // 1. Close container tar stdin
                                    let _ = import.tar_stdin.shutdown().await;

                                    // 2. Wait for container tar to finish
                                    let status = import.tar_child.wait().await;
                                    match &status {
                                        Ok(s) if s.success() => {
                                            info!(transfer_id = %transfer_id, "Nspawn container tar extraction succeeded");
                                        }
                                        Ok(s) => {
                                            error!(transfer_id = %transfer_id, "Nspawn container tar extraction failed: {}", s);
                                            let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::ImportFailed {
                                                transfer_id: transfer_id.clone(),
                                                error: format!("Container tar extraction failed: {}", s),
                                            })).await;
                                            active_nspawn_imports.remove(&transfer_id);
                                            continue;
                                        }
                                        Err(e) => {
                                            error!(transfer_id = %transfer_id, "Wait for nspawn container tar: {}", e);
                                            let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::ImportFailed {
                                                transfer_id: transfer_id.clone(),
                                                error: format!("Container tar wait error: {}", e),
                                            })).await;
                                            active_nspawn_imports.remove(&transfer_id);
                                            continue;
                                        }
                                    }

                                    // 3. Create workspace directory
                                    let ws_dir = format!("{}/{}-workspace", import.storage_path, import.container_name);
                                    if let Err(e) = tokio::fs::create_dir_all(&ws_dir).await {
                                        error!("Failed to create nspawn workspace dir: {}", e);
                                        let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::ImportFailed {
                                            transfer_id: transfer_id.clone(),
                                            error: format!("Failed to create workspace dir: {}", e),
                                        })).await;
                                        active_nspawn_imports.remove(&transfer_id);
                                        continue;
                                    }

                                    // 4. Spawn workspace tar
                                    match tokio::process::Command::new("tar")
                                        .args(["xf", "-", "--numeric-owner", "--xattrs", "--xattrs-include=*", "-C", &ws_dir])
                                        .stdin(std::process::Stdio::piped())
                                        .stdout(std::process::Stdio::null())
                                        .stderr(std::process::Stdio::piped())
                                        .spawn()
                                    {
                                        Ok(mut ws_child) => {
                                            let ws_stdin = ws_child.stdin.take().expect("ws tar stdin");
                                            import.ws_tar_child = Some(ws_child);
                                            import.ws_tar_stdin = Some(ws_stdin);
                                            import.phase = ImportPhase::ReceivingWorkspace;
                                        }
                                        Err(e) => {
                                            error!("Failed to spawn nspawn workspace tar: {}", e);
                                            let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::ImportFailed {
                                                transfer_id: transfer_id.clone(),
                                                error: format!("Failed to spawn workspace tar: {}", e),
                                            })).await;
                                            active_nspawn_imports.remove(&transfer_id);
                                        }
                                    }
                                }
                            }
                            Ok(HostRegistryMessage::TransferComplete { transfer_id }) => {
                                if let Some(mut import) = active_nspawn_imports.remove(&transfer_id) {
                                    use tokio::io::AsyncWriteExt;

                                    let container_name = import.container_name.clone();
                                    let storage_path = import.storage_path.clone();
                                    let network_mode = import.network_mode.clone();

                                    if import.phase == ImportPhase::ReceivingWorkspace {
                                        // Close workspace tar
                                        if let Some(mut ws_stdin) = import.ws_tar_stdin.take() {
                                            let _ = ws_stdin.shutdown().await;
                                        }
                                        if let Some(mut ws_child) = import.ws_tar_child.take() {
                                            match ws_child.wait().await {
                                                Ok(s) if s.success() => {
                                                    info!(transfer_id = %transfer_id, "Nspawn workspace tar extraction succeeded");
                                                }
                                                Ok(s) => {
                                                    warn!(transfer_id = %transfer_id, "Nspawn workspace tar extraction failed: {}, creating empty workspace", s);
                                                    let ws_dir = format!("{}/{}-workspace", storage_path, container_name);
                                                    let _ = tokio::fs::create_dir_all(&ws_dir).await;
                                                }
                                                Err(e) => {
                                                    warn!(transfer_id = %transfer_id, "Nspawn workspace tar wait error: {}, creating empty workspace", e);
                                                    let ws_dir = format!("{}/{}-workspace", storage_path, container_name);
                                                    let _ = tokio::fs::create_dir_all(&ws_dir).await;
                                                }
                                            }
                                        }
                                    } else {
                                        // No workspace phase -- close container tar
                                        let _ = import.tar_stdin.shutdown().await;
                                        match import.tar_child.wait().await {
                                            Ok(s) if s.success() => {
                                                info!(transfer_id = %transfer_id, "Nspawn container tar extraction succeeded");
                                            }
                                            Ok(s) => {
                                                let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::ImportFailed {
                                                    transfer_id: transfer_id.clone(),
                                                    error: format!("Container tar extraction failed: {}", s),
                                                })).await;
                                                continue;
                                            }
                                            Err(e) => {
                                                let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::ImportFailed {
                                                    transfer_id: transfer_id.clone(),
                                                    error: format!("Container tar wait error: {}", e),
                                                })).await;
                                                continue;
                                            }
                                        }
                                        // Create empty workspace
                                        let ws_dir = format!("{}/{}-workspace", storage_path, container_name);
                                        let _ = tokio::fs::create_dir_all(&ws_dir).await;
                                    }

                                    // Finalize nspawn import: write .nspawn unit, network config, start
                                    let tx_finalize = tx.clone();
                                    let tid = transfer_id.clone();
                                    tokio::spawn(async move {
                                        let sp = std::path::Path::new(&storage_path);

                                        // Write .nspawn unit
                                        if let Err(e) = hr_container::NspawnClient::write_nspawn_unit(&container_name, sp, &network_mode).await {
                                            let _ = tx_finalize.send(OutgoingWsMessage::Text(HostAgentMessage::ImportFailed {
                                                transfer_id: tid,
                                                error: format!("Failed to write nspawn unit: {}", e),
                                            })).await;
                                            return;
                                        }

                                        // Write network config in rootfs
                                        if let Err(e) = hr_container::NspawnClient::write_network_config(&container_name, sp).await {
                                            let _ = tx_finalize.send(OutgoingWsMessage::Text(HostAgentMessage::ImportFailed {
                                                transfer_id: tid,
                                                error: format!("Failed to write network config: {}", e),
                                            })).await;
                                            return;
                                        }

                                        // Start the container
                                        match hr_container::NspawnClient::start_container(&container_name).await {
                                            Ok(()) => {
                                                info!(transfer_id = %tid, "Nspawn import complete, container started");
                                                let _ = tx_finalize.send(OutgoingWsMessage::Text(HostAgentMessage::ImportComplete {
                                                    transfer_id: tid,
                                                    container_name,
                                                })).await;
                                            }
                                            Err(e) => {
                                                let _ = tx_finalize.send(OutgoingWsMessage::Text(HostAgentMessage::ImportFailed {
                                                    transfer_id: tid,
                                                    error: format!("Container start failed: {}", e),
                                                })).await;
                                            }
                                        }
                                    });
                                }
                            }
                            Ok(HostRegistryMessage::DeleteContainer { container_name }) => {
                                info!(container = %container_name, "Deleting container");
                                let _ = tokio::process::Command::new("lxc")
                                    .args(["delete", &container_name, "--force"])
                                    .output()
                                    .await;
                                // Also delete workspace storage volume
                                let vol_name = format!("{container_name}-workspace");
                                let _ = tokio::process::Command::new("lxc")
                                    .args(["storage", "volume", "delete", "default", &vol_name])
                                    .output()
                                    .await;
                            }
                            Ok(HostRegistryMessage::StartContainer { container_name }) => {
                                info!(container = %container_name, "Starting container");
                                let _ = tokio::process::Command::new("lxc")
                                    .args(["start", &container_name])
                                    .output()
                                    .await;
                            }
                            Ok(HostRegistryMessage::StopContainer { container_name }) => {
                                info!(container = %container_name, "Stopping container");
                                let _ = tokio::process::Command::new("lxc")
                                    .args(["stop", &container_name, "--force"])
                                    .output()
                                    .await;
                            }
                            Ok(HostRegistryMessage::ExecInContainer { request_id, container_name, command }) => {
                                info!(container = %container_name, "Executing command in container");
                                let tx_exec = tx.clone();
                                tokio::spawn(async move {
                                    let mut lxc_args = vec!["exec".to_string(), container_name, "--".to_string()];
                                    lxc_args.extend(command);
                                    let lxc_refs: Vec<&str> = lxc_args.iter().map(|s| s.as_str()).collect();
                                    let result = tokio::process::Command::new("lxc")
                                        .args(&lxc_refs)
                                        .output()
                                        .await;
                                    let (success, stdout, stderr) = match result {
                                        Ok(out) => (
                                            out.status.success(),
                                            String::from_utf8_lossy(&out.stdout).to_string(),
                                            String::from_utf8_lossy(&out.stderr).to_string(),
                                        ),
                                        Err(e) => (false, String::new(), e.to_string()),
                                    };
                                    let _ = tx_exec.send(OutgoingWsMessage::Text(HostAgentMessage::ExecResult {
                                        request_id,
                                        success,
                                        stdout,
                                        stderr,
                                    })).await;
                                });
                            }
                            Ok(HostRegistryMessage::CreateContainer { .. }) => {
                                warn!("CreateContainer not yet implemented");
                            }
                            Ok(HostRegistryMessage::PushAgentUpdate { version, download_url, sha256 }) => {
                                info!(version = %version, "Agent update received, starting self-update");
                                tokio::spawn(async move {
                                    if let Err(e) = self_update(&download_url, &sha256).await {
                                        error!("Self-update failed: {}", e);
                                    }
                                });
                            }
                            Ok(HostRegistryMessage::PowerOff) => {
                                info!("Poweroff requested via agent");
                                tokio::spawn(async {
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                    let _ = tokio::process::Command::new("sudo")
                                        .args(["poweroff"])
                                        .output()
                                        .await;
                                });
                            }
                            Ok(HostRegistryMessage::Reboot) => {
                                info!("Reboot requested via agent");
                                tokio::spawn(async {
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                    let _ = tokio::process::Command::new("sudo")
                                        .args(["reboot"])
                                        .output()
                                        .await;
                                });
                            }
                            Ok(HostRegistryMessage::SuspendHost) => {
                                info!("Suspend requested via agent");
                                tokio::spawn(async {
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                    let _ = tokio::process::Command::new("sudo")
                                        .args(["systemctl", "suspend"])
                                        .output()
                                        .await;
                                });
                            }
                            Ok(HostRegistryMessage::SetAutoOff { mode, minutes }) => {
                                info!(?mode, minutes, "Auto-off configured");
                                auto_off_mode = Some(mode);
                                auto_off_minutes = minutes;
                                idle_since = None;
                            }
                            Ok(HostRegistryMessage::CancelTransfer { transfer_id }) => {
                                info!(transfer_id = %transfer_id, "Transfer cancelled");
                                if let Some(mut import) = active_nspawn_imports.remove(&transfer_id) {
                                    // Kill container tar
                                    let _ = import.tar_child.kill().await;
                                    drop(import.tar_stdin);
                                    // Kill workspace tar if active
                                    if let Some(mut ws_child) = import.ws_tar_child.take() {
                                        let _ = ws_child.kill().await;
                                    }
                                    if let Some(ws_stdin) = import.ws_tar_stdin.take() {
                                        drop(ws_stdin);
                                    }
                                    // Clean up extracted nspawn files
                                    let rootfs_dir = format!("{}/{}", import.storage_path, import.container_name);
                                    let _ = tokio::fs::remove_dir_all(&rootfs_dir).await;
                                    let ws_dir = format!("{}/{}-workspace", import.storage_path, import.container_name);
                                    let _ = tokio::fs::remove_dir_all(&ws_dir).await;
                                    info!(transfer_id = %transfer_id, "Cleaned up cancelled nspawn import");
                                }
                                if let Some((ref tid, _)) = pending_binary_chunk {
                                    if tid == &transfer_id {
                                        pending_binary_chunk = None;
                                    }
                                }
                            }
                            // ── Nspawn container handlers ──────────────────
                            Ok(HostRegistryMessage::CreateNspawnContainer {
                                app_id: _, slug: _, container_name, storage_path, bridge,
                                agent_token: _, agent_config: _,
                            }) => {
                                info!(container = %container_name, storage = %storage_path, "Creating nspawn container");
                                tokio::spawn(async move {
                                    let sp = std::path::Path::new(&storage_path);
                                    let _network_mode = format!("bridge:{bridge}");
                                    match hr_container::NspawnClient::create_container(&container_name, sp).await {
                                        Ok(()) => {
                                            info!(container = %container_name, "Nspawn container created successfully");
                                        }
                                        Err(e) => {
                                            error!(container = %container_name, "Nspawn container creation failed: {e}");
                                        }
                                    }
                                });
                            }
                            Ok(HostRegistryMessage::DeleteNspawnContainer { container_name, storage_path }) => {
                                info!(container = %container_name, "Deleting nspawn container");
                                tokio::spawn(async move {
                                    let sp = std::path::Path::new(&storage_path);
                                    if let Err(e) = hr_container::NspawnClient::delete_container(&container_name, sp).await {
                                        error!(container = %container_name, "Nspawn delete failed: {e}");
                                    }
                                });
                            }
                            Ok(HostRegistryMessage::StartNspawnContainer { container_name, storage_path: _ }) => {
                                info!(container = %container_name, "Starting nspawn container");
                                tokio::spawn(async move {
                                    if let Err(e) = hr_container::NspawnClient::start_container(&container_name).await {
                                        error!(container = %container_name, "Nspawn start failed: {e}");
                                    }
                                });
                            }
                            Ok(HostRegistryMessage::StopNspawnContainer { container_name }) => {
                                info!(container = %container_name, "Stopping nspawn container");
                                tokio::spawn(async move {
                                    if let Err(e) = hr_container::NspawnClient::stop_container(&container_name).await {
                                        error!(container = %container_name, "Nspawn stop failed: {e}");
                                    }
                                });
                            }
                            Ok(HostRegistryMessage::ExecInNspawnContainer { request_id, container_name, command }) => {
                                info!(container = %container_name, "Executing command in nspawn container");
                                let tx_exec = tx.clone();
                                tokio::spawn(async move {
                                    let cmd_refs: Vec<&str> = command.iter().map(|s| s.as_str()).collect();
                                    let (success, stdout, stderr) = match hr_container::NspawnClient::exec(&container_name, &cmd_refs).await {
                                        Ok(out) => (true, out, String::new()),
                                        Err(e) => (false, String::new(), e.to_string()),
                                    };
                                    let _ = tx_exec.send(OutgoingWsMessage::Text(HostAgentMessage::ExecResult {
                                        request_id,
                                        success,
                                        stdout,
                                        stderr,
                                    })).await;
                                });
                            }
                            Ok(HostRegistryMessage::StartNspawnExport { container_name, storage_path, transfer_id }) => {
                                info!(container = %container_name, transfer_id = %transfer_id, "Starting nspawn export");
                                let tx_export = tx.clone();
                                tokio::spawn(async move {
                                    handle_nspawn_export(tx_export, transfer_id, container_name, storage_path).await;
                                });
                            }
                            Ok(HostRegistryMessage::StartNspawnImport { container_name, storage_path, transfer_id, network_mode }) => {
                                info!(container = %container_name, transfer_id = %transfer_id, "Preparing nspawn import");

                                let rootfs_dir = format!("{}/{}", storage_path, container_name);

                                // Create target directory
                                if let Err(e) = tokio::fs::create_dir_all(&rootfs_dir).await {
                                    error!("Failed to create rootfs dir: {}", e);
                                    let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::ImportFailed {
                                        transfer_id, error: format!("Failed to create rootfs dir: {}", e),
                                    })).await;
                                    continue;
                                }

                                // Spawn tar to extract incoming rootfs data
                                match tokio::process::Command::new("tar")
                                    .args(["xf", "-", "--numeric-owner", "--xattrs", "--xattrs-include=*", "-C", &rootfs_dir])
                                    .stdin(std::process::Stdio::piped())
                                    .stdout(std::process::Stdio::null())
                                    .stderr(std::process::Stdio::piped())
                                    .spawn()
                                {
                                    Ok(mut child) => {
                                        let stdin = child.stdin.take().expect("tar stdin");
                                        active_nspawn_imports.insert(transfer_id, ActiveNspawnImport {
                                            container_name,
                                            storage_path,
                                            tar_child: child,
                                            tar_stdin: stdin,
                                            phase: ImportPhase::ReceivingContainer,
                                            ws_tar_child: None,
                                            ws_tar_stdin: None,
                                            network_mode,
                                        });
                                    }
                                    Err(e) => {
                                        let _ = tokio::fs::remove_dir_all(&rootfs_dir).await;
                                        let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::ImportFailed {
                                            transfer_id, error: format!("Failed to spawn tar: {}", e),
                                        })).await;
                                    }
                                }
                            }
                            Ok(HostRegistryMessage::AuthResult { .. }) => {
                                // Already handled during auth phase
                            }
                            Err(e) => {
                                warn!("Failed to parse message: {}", e);
                            }
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        if let Some((transfer_id, expected_checksum)) = pending_binary_chunk.take() {
                            let actual_checksum = xxhash_rust::xxh32::xxh32(&data, 0);
                            if actual_checksum != expected_checksum {
                                warn!(
                                    transfer_id = %transfer_id,
                                    expected = expected_checksum,
                                    actual = actual_checksum,
                                    "Binary chunk checksum mismatch, skipping"
                                );
                            } else if let Some(import) = active_nspawn_imports.get_mut(&transfer_id) {
                                // Nspawn import
                                use tokio::io::AsyncWriteExt;
                                let target = match import.phase {
                                    ImportPhase::ReceivingWorkspace => import.ws_tar_stdin.as_mut().unwrap_or(&mut import.tar_stdin),
                                    ImportPhase::ReceivingContainer => &mut import.tar_stdin,
                                };
                                if let Err(e) = target.write_all(&data).await {
                                    error!("Failed to write binary chunk for {}: {}", transfer_id, e);
                                }
                            } else {
                                warn!(transfer_id = %transfer_id, "Binary chunk for unknown import");
                            }
                        } else {
                            warn!("Unexpected binary WebSocket frame (no pending metadata)");
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = write.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!("WebSocket closed");
                        break;
                    }
                    Some(Err(e)) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
            // Auto-off idle monitoring (sleep or shutdown)
            Ok(()) = cpu_rx.changed() => {
                let mode = match auto_off_mode {
                    Some(m) if auto_off_minutes > 0 => m,
                    _ => {
                        idle_since = None;
                        continue;
                    }
                };
                let cpu = *cpu_rx.borrow();
                if cpu < CPU_IDLE_THRESHOLD {
                    if idle_since.is_none() {
                        info!(cpu_percent = cpu, timeout_minutes = auto_off_minutes, ?mode,
                              "Host entering idle state, starting auto-off countdown");
                        idle_since = Some(tokio::time::Instant::now());
                    }
                    if let Some(since) = idle_since {
                        let idle_mins = since.elapsed().as_secs() / 60;
                        if idle_mins >= auto_off_minutes as u64 {
                            info!(idle_minutes = idle_mins, ?mode,
                                  "Idle timeout reached, executing auto-off");
                            let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::AutoOffNotify { mode })).await;
                            let cmd_args: &[&str] = match mode {
                                AutoOffMode::Sleep => &["systemctl", "suspend"],
                                AutoOffMode::Shutdown => &["poweroff"],
                            };
                            let args: Vec<String> = cmd_args.iter().map(|s| s.to_string()).collect();
                            tokio::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                let _ = tokio::process::Command::new("sudo")
                                    .args(&args)
                                    .output()
                                    .await;
                            });
                            idle_since = None;
                        }
                    }
                } else {
                    if idle_since.is_some() {
                        info!(cpu_percent = cpu, "Host no longer idle, resetting auto-off countdown");
                    }
                    idle_since = None;
                }
            }
        }
    }

    // Clean up orphaned nspawn imports on disconnect
    for (tid, mut import) in active_nspawn_imports {
        warn!(transfer_id = %tid, "Cleaning orphaned nspawn import on disconnect");
        let _ = import.tar_child.kill().await;
        drop(import.tar_stdin);
        if let Some(mut ws_child) = import.ws_tar_child.take() {
            let _ = ws_child.kill().await;
        }
        if let Some(ws_stdin) = import.ws_tar_stdin.take() {
            drop(ws_stdin);
        }
        let rootfs_dir = format!("{}/{}", import.storage_path, import.container_name);
        let _ = tokio::fs::remove_dir_all(&rootfs_dir).await;
        let ws_dir = format!("{}/{}-workspace", import.storage_path, import.container_name);
        let _ = tokio::fs::remove_dir_all(&ws_dir).await;
    }

    heartbeat_handle.abort();
    metrics_handle.abort();
    ifaces_handle.abort();
    Ok(())
}

/// Handle nspawn container export (stop + tar rootfs + workspace).
async fn handle_nspawn_export(
    tx: tokio::sync::mpsc::Sender<OutgoingWsMessage>,
    transfer_id: String,
    container_name: String,
    storage_path: String,
) {
    // 1. Stop container
    info!(container = %container_name, "Stopping nspawn container for export");
    if let Err(e) = hr_container::NspawnClient::stop_container(&container_name).await {
        let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::ExportFailed {
            transfer_id, error: format!("Failed to stop container: {}", e),
        })).await;
        return;
    }

    // Wait for container to fully stop
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // 2. Build paths
    let rootfs_dir = format!("{}/{}", storage_path, container_name);
    let workspace_dir = format!("{}/{}-workspace", storage_path, container_name);

    // 3. Estimate container size
    let estimated_size = estimate_dir_size(&rootfs_dir).await;

    // 4. Send ExportReady
    let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::ExportReady {
        transfer_id: transfer_id.clone(),
        container_name: container_name.clone(),
        size_bytes: estimated_size,
    })).await;

    // 5. Stream container tar
    if let Err(e) = stream_tar_export(&tx, &transfer_id, &rootfs_dir, estimated_size).await {
        let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::ExportFailed {
            transfer_id, error: e,
        })).await;
        return;
    }

    // 6. Stream workspace if directory exists
    let ws_path = std::path::Path::new(&workspace_dir);
    if ws_path.exists() {
        let ws_size = estimate_dir_size(&workspace_dir).await;
        let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::WorkspaceReady {
            transfer_id: transfer_id.clone(),
            size_bytes: ws_size,
        })).await;

        if let Err(e) = stream_tar_export(&tx, &transfer_id, &workspace_dir, ws_size).await {
            warn!(container = %container_name, "Nspawn workspace export failed (non-fatal): {}", e);
        }
    }

    // 7. Send TransferComplete
    let _ = tx.send(OutgoingWsMessage::Text(HostAgentMessage::TransferComplete {
        transfer_id: transfer_id.clone(),
    })).await;

    info!(transfer_id = %transfer_id, "Nspawn export complete");
}

async fn estimate_dir_size(dir: &str) -> u64 {
    match tokio::process::Command::new("du")
        .args(["-sb", dir])
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.split_whitespace().next()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        }
        _ => 0,
    }
}

/// Stream a directory via tar to the WebSocket channel.
async fn stream_tar_export(
    tx: &tokio::sync::mpsc::Sender<OutgoingWsMessage>,
    transfer_id: &str,
    dir_path: &str,
    estimated_size: u64,
) -> Result<(), String> {
    use tokio::io::AsyncReadExt;

    let mut child = tokio::process::Command::new("tar")
        .args(["cf", "-", "--numeric-owner", "--xattrs", "--xattrs-include=*", "-C", dir_path, "."])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn tar: {e}"))?;

    let mut stdout = child.stdout.take()
        .ok_or_else(|| "Failed to get tar stdout".to_string())?;

    let mut buf = vec![0u8; 524288]; // 512KB
    let mut sequence: u32 = 0;
    let mut send_failed = false;
    let mut total_sent: u64 = 0;

    loop {
        let n = match stdout.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                child.kill().await.ok();
                return Err(format!("Read error from tar stdout: {e}"));
            }
        };

        let checksum = xxhash_rust::xxh32::xxh32(&buf[..n], 0);

        if tx.send(OutgoingWsMessage::Text(HostAgentMessage::TransferChunkBinary {
            transfer_id: transfer_id.to_string(),
            sequence,
            size: n as u32,
            checksum,
        })).await.is_err() {
            send_failed = true;
            break;
        }

        if tx.send(OutgoingWsMessage::Binary(buf[..n].to_vec())).await.is_err() {
            send_failed = true;
            break;
        }

        sequence += 1;
        total_sent += n as u64;

        if sequence % 4 == 0 && estimated_size > 0 {
            info!(
                transfer_id = %transfer_id,
                sent_bytes = total_sent,
                estimated_bytes = estimated_size,
                "Export progress: {:.1}%",
                (total_sent as f64 / estimated_size as f64 * 100.0).min(100.0)
            );
        }
    }

    let status = child.wait().await
        .map_err(|e| format!("Wait for tar: {e}"))?;

    if send_failed {
        return Err("Transfer channel closed during export".to_string());
    }

    if !status.success() {
        return Err(format!("tar exited with status: {}", status));
    }

    info!(transfer_id = %transfer_id, total_bytes = total_sent, "Tar export stream complete");
    Ok(())
}

async fn self_update(download_url: &str, expected_sha256: &str) -> Result<(), String> {
    use sha2::{Sha256, Digest};

    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Cannot determine current exe: {}", e))?;
    let tmp_path = format!("{}.new", current_exe.display());

    info!(url = download_url, "Downloading new binary");
    let output = tokio::process::Command::new("curl")
        .args(["-fsSL", "-o", &tmp_path, download_url])
        .output()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    if !output.status.success() {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(format!("curl failed: {}", String::from_utf8_lossy(&output.stderr)));
    }

    let data = std::fs::read(&tmp_path)
        .map_err(|e| format!("Read downloaded binary: {}", e))?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let actual_sha256 = hex::encode(hasher.finalize());

    if actual_sha256 != expected_sha256 {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(format!("SHA256 mismatch: expected {}, got {}", expected_sha256, actual_sha256));
    }

    let _ = tokio::process::Command::new("chmod")
        .args(["+x", &tmp_path])
        .output()
        .await;

    tokio::fs::rename(&tmp_path, &current_exe)
        .await
        .map_err(|e| format!("Rename failed: {}", e))?;

    info!("Binary replaced, restarting via systemd");
    let _ = tokio::process::Command::new("sudo")
        .args(["systemctl", "restart", "hr-host-agent"])
        .output()
        .await;

    Ok(())
}

fn collect_interfaces() -> Vec<hr_registry::protocol::NetworkInterfaceInfo> {
    let mut interfaces = Vec::new();
    let entries = match std::fs::read_dir("/sys/class/net") {
        Ok(e) => e,
        Err(_) => return interfaces,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "lo" { continue; }
        let mac = std::fs::read_to_string(format!("/sys/class/net/{}/address", name))
            .unwrap_or_default().trim().to_string();
        if mac.is_empty() || mac == "00:00:00:00:00:00" { continue; }
        let operstate = std::fs::read_to_string(format!("/sys/class/net/{}/operstate", name))
            .unwrap_or_default().trim().to_string();
        // Get IPv4 address
        let ipv4 = std::process::Command::new("ip")
            .args(["-4", "-o", "addr", "show", &name])
            .output()
            .ok()
            .and_then(|o| {
                let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                stdout.split_whitespace()
                    .find(|s| s.contains('/'))
                    .map(|s| s.split('/').next().unwrap_or("").to_string())
            })
            .filter(|s| !s.is_empty());
        interfaces.push(hr_registry::protocol::NetworkInterfaceInfo {
            name, mac, ipv4, is_up: operstate == "up",
        });
    }
    interfaces
}

fn collect_metrics() -> HostMetrics {
    // Read /proc/meminfo
    let (mem_total, mem_available) = {
        let content = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let mut total = 0u64;
        let mut available = 0u64;
        for line in content.lines() {
            if let Some(val) = line.strip_prefix("MemTotal:") {
                total = val
                    .trim()
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
                    * 1024;
            }
            if let Some(val) = line.strip_prefix("MemAvailable:") {
                available = val
                    .trim()
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
                    * 1024;
            }
        }
        (total, available)
    };

    // Read /proc/loadavg
    let load_avg = {
        let content = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
        let parts: Vec<f32> = content
            .split_whitespace()
            .take(3)
            .filter_map(|s| s.parse().ok())
            .collect();
        [
            parts.first().copied().unwrap_or(0.0),
            parts.get(1).copied().unwrap_or(0.0),
            parts.get(2).copied().unwrap_or(0.0),
        ]
    };

    // Disk usage for /
    let (disk_total, disk_used) = {
        let output = std::process::Command::new("df")
            .args(["-B1", "/"])
            .output()
            .ok();
        match output {
            Some(o) if o.status.success() => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let line = stdout.lines().nth(1).unwrap_or("");
                let parts: Vec<&str> = line.split_whitespace().collect();
                let total = parts
                    .get(1)
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                let used = parts
                    .get(2)
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                (total, used)
            }
            _ => (0, 0),
        }
    };

    HostMetrics {
        cpu_percent: load_avg[0] * 100.0 / num_cpus().max(1) as f32,
        memory_used_bytes: mem_total.saturating_sub(mem_available),
        memory_total_bytes: mem_total,
        disk_used_bytes: disk_used,
        disk_total_bytes: disk_total,
        load_avg,
    }
}

fn num_cpus() -> usize {
    std::fs::read_to_string("/proc/cpuinfo")
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("processor"))
        .count()
        .max(1)
}
