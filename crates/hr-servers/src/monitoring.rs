use hr_common::events::ServerStatusEvent;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, error, info};

const SERVERS_FILE: &str = "/data/servers.json";
const MONITOR_INTERVAL_SECS: u64 = 30;

/// Run the server monitoring loop.
/// Pings all servers every 30 seconds and updates their status in servers.json.
/// Emits ServerStatusEvent on the event bus for each status change.
pub async fn run_monitoring(events: Arc<broadcast::Sender<ServerStatusEvent>>) {
    info!("Server monitoring started (interval: {}s)", MONITOR_INTERVAL_SECS);

    loop {
        if let Err(e) = monitor_all_servers(&events).await {
            error!("Monitoring cycle error: {}", e);
        }
        tokio::time::sleep(std::time::Duration::from_secs(MONITOR_INTERVAL_SECS)).await;
    }
}

async fn monitor_all_servers(
    events: &broadcast::Sender<ServerStatusEvent>,
) -> Result<(), String> {
    let content = match tokio::fs::read_to_string(SERVERS_FILE).await {
        Ok(c) => c,
        Err(_) => return Ok(()), // No servers file yet
    };

    let mut data: Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    let servers = match data.get_mut("servers").and_then(|s| s.as_array_mut()) {
        Some(s) => s,
        None => return Ok(()),
    };

    if servers.is_empty() {
        return Ok(());
    }

    // Ping all servers in parallel
    let mut join_set = tokio::task::JoinSet::new();
    for server in servers.iter() {
        let host = server
            .get("host")
            .and_then(|h| h.as_str())
            .unwrap_or("")
            .to_string();
        let id = server
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or("")
            .to_string();

        if host.is_empty() || id.is_empty() {
            continue;
        }

        join_set.spawn(async move {
            let (status, latency) = ping_host(&host).await;
            (id, status, latency)
        });
    }

    let mut results: Vec<(String, String, Option<u64>)> = Vec::new();
    while let Some(Ok(result)) = join_set.join_next().await {
        results.push(result);
    }

    // Update statuses
    let now = chrono::Utc::now().to_rfc3339();

    for (id, new_status, latency) in &results {
        if let Some(server) = servers.iter_mut().find(|s| {
            s.get("id").and_then(|i| i.as_str()) == Some(id)
        }) {
            let old_status = server
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");

            if old_status != new_status {
                debug!("{}: {} -> {}", id, old_status, new_status);
            }

            server["status"] = serde_json::json!(new_status);
            server["latency"] = serde_json::json!(latency.unwrap_or(0));
            server["lastSeen"] = serde_json::json!(&now);

            // Emit status event
            let _ = events.send(ServerStatusEvent {
                server_id: id.clone(),
                status: new_status.clone(),
                latency_ms: *latency,
            });
        }
    }

    // Always save to persist lastSeen and latency updates
    if !results.is_empty() {
        let content = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
        let tmp = format!("{}.tmp", SERVERS_FILE);
        tokio::fs::write(&tmp, &content)
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::rename(&tmp, SERVERS_FILE)
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Ping a host and return (status, latency_ms).
async fn ping_host(host: &str) -> (String, Option<u64>) {
    let start = std::time::Instant::now();
    let output = tokio::process::Command::new("ping")
        .args(["-c", "1", "-W", "5", host])
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => {
            let latency = start.elapsed().as_millis() as u64;
            ("online".to_string(), Some(latency))
        }
        _ => ("offline".to_string(), None),
    }
}
