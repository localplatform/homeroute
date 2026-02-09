//! WebSocket client connecting to HomeRoute registry.

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use hr_registry::protocol::{AgentMessage, RegistryMessage};

use crate::config::AgentConfig;

/// Connect to HomeRoute, authenticate, and handle bidirectional communication.
/// - `registry_tx`: Channel to send received RegistryMessages to the main loop.
/// - `outbound_rx`: Channel to receive AgentMessages to send to the registry (metrics, etc.).
pub async fn run_connection(
    config: &AgentConfig,
    registry_tx: mpsc::Sender<RegistryMessage>,
    mut outbound_rx: mpsc::Receiver<AgentMessage>,
) -> Result<()> {
    let url = config.ws_url();
    info!(url, "Connecting to HomeRoute registry");

    let (ws_stream, _response) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| anyhow::anyhow!("WebSocket connect failed: {e}"))?;

    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // Detect our IPv4 address on the configured interface
    let ipv4_address = detect_ipv4_address(&config.interface).await;
    if let Some(ref addr) = ipv4_address {
        info!(addr, interface = %config.interface, "Detected IPv4 address");
    } else {
        warn!(interface = %config.interface, "No IPv4 address detected");
    }

    // Send Auth message
    let auth_msg = AgentMessage::Auth {
        token: config.token.clone(),
        service_name: config.service_name.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        ipv4_address,
    };
    let auth_json = serde_json::to_string(&auth_msg)?;
    ws_sink.send(Message::Text(auth_json.into())).await?;

    info!("Auth message sent, waiting for response");

    // Wait for AuthResult
    let first_msg = ws_stream
        .next()
        .await
        .ok_or_else(|| anyhow::anyhow!("Connection closed before auth response"))??;

    let auth_result: RegistryMessage = match first_msg {
        Message::Text(text) => serde_json::from_str(&text)?,
        other => anyhow::bail!("Unexpected message type during auth: {other:?}"),
    };

    match auth_result {
        RegistryMessage::AuthResult { success: true, .. } => {
            info!("Authentication successful");
        }
        RegistryMessage::AuthResult { success: false, error, .. } => {
            anyhow::bail!("Authentication failed: {}", error.unwrap_or_default());
        }
        _ => anyhow::bail!("Unexpected message during auth handshake"),
    }

    // Start heartbeat task
    let (heartbeat_tx, mut heartbeat_rx) = mpsc::channel::<()>(1);
    let start_time = std::time::Instant::now();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            if heartbeat_tx.send(()).await.is_err() {
                break;
            }
        }
    });

    // Main message loop
    loop {
        tokio::select! {
            // Incoming messages from registry
            ws_msg = ws_stream.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<RegistryMessage>(&text) {
                            Ok(msg) => {
                                let is_shutdown = matches!(&msg, RegistryMessage::Shutdown);
                                if registry_tx.send(msg).await.is_err() {
                                    error!("Registry message channel closed");
                                    break;
                                }
                                if is_shutdown {
                                    info!("Shutdown requested by registry");
                                    break;
                                }
                            }
                            Err(e) => {
                                warn!("Invalid message from registry: {e}");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!("WebSocket connection closed");
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = ws_sink.send(Message::Pong(data)).await;
                    }
                    Some(Err(e)) => {
                        error!("WebSocket error: {e}");
                        break;
                    }
                    _ => {}
                }
            }

            // Outbound messages (metrics, service state changes, etc.)
            Some(agent_msg) = outbound_rx.recv() => {
                let json = match serde_json::to_string(&agent_msg) {
                    Ok(j) => j,
                    Err(e) => {
                        warn!("Failed to serialize agent message: {e}");
                        continue;
                    }
                };
                if ws_sink.send(Message::Text(json.into())).await.is_err() {
                    error!("Failed to send agent message");
                    break;
                }
            }

            // Heartbeat timer
            Some(()) = heartbeat_rx.recv() => {
                let uptime = start_time.elapsed().as_secs();
                let hb = AgentMessage::Heartbeat {
                    uptime_secs: uptime,
                    connections_active: 0,
                };
                let json = serde_json::to_string(&hb)?;
                if ws_sink.send(Message::Text(json.into())).await.is_err() {
                    error!("Failed to send heartbeat");
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Detect the IPv4 address on the given interface (for local DNS A records).
async fn detect_ipv4_address(interface: &str) -> Option<String> {
    let output = tokio::process::Command::new("ip")
        .args(["-4", "-o", "addr", "show", "dev", interface, "scope", "global"])
        .output()
        .await
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if let Some(addr_idx) = parts.iter().position(|&p| p == "inet") {
            if let Some(addr_cidr) = parts.get(addr_idx + 1) {
                let addr = addr_cidr.split('/').next().unwrap_or(addr_cidr);
                if addr.starts_with("127.") || addr.starts_with("169.254.") {
                    continue;
                }
                return Some(addr.to_string());
            }
        }
    }
    None
}
