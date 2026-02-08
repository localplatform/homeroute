use axum::{
    extract::{Path, State, WebSocketUpgrade, ws::{Message, WebSocket}},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::state::ApiState;

const HOSTS_FILE: &str = "/data/hosts.json";
const SSH_KEY_PATH: &str = "/data/ssh/id_rsa";
const SSH_PUB_KEY_PATH: &str = "/data/ssh/id_rsa.pub";
const HOST_AGENT_BINARY: &str = "/opt/homeroute/data/agent-binaries/hr-host-agent";
const HOMEROUTE_LAN_IP: &str = "10.0.0.254";
const API_PORT: u16 = 4000;

pub fn router() -> Router<ApiState> {
    Router::new()
        // Host CRUD
        .route("/", get(list_hosts).post(add_host))
        .route("/groups", get(list_groups))
        // Agent routes (must be before /{id} to avoid path conflicts)
        .route("/agents/update", post(update_host_agents))
        .route("/agents/binary", get(serve_host_agent_binary))
        .route("/{id}", get(get_host).put(update_host).delete(delete_host))
        // Connection
        .route("/{id}/test", post(test_connection))
        .route("/{id}/info", post(get_host_info))
        // Power actions
        .route("/{id}/wake", post(wake))
        .route("/{id}/shutdown", post(shutdown_host))
        .route("/{id}/reboot", post(reboot_host))
        .route("/{id}/sleep", post(sleep_host))
        .route("/{id}/wol-mac", post(set_wol_mac))
        .route("/{id}/auto-off", post(set_auto_off))
        .route("/{id}/metrics", get(get_host_metrics))
        .route("/bulk/wake", post(bulk_wake))
        .route("/bulk/shutdown", post(bulk_shutdown))
        // Container management on remote hosts
        .route("/{id}/containers/{name}/start", post(start_container))
        .route("/{id}/containers/{name}/stop", post(stop_container))
        .route("/{id}/containers/{name}/delete", post(delete_container))
        .route("/{id}/exec", post(exec_on_host))
        // Host-agent WebSocket
        .route("/agent/ws", get(host_agent_ws))
}

// ── Data access ──────────────────────────────────────────────────────────

async fn load_hosts() -> Value {
    match tokio::fs::read_to_string(HOSTS_FILE).await {
        Ok(content) => serde_json::from_str(&content).unwrap_or(json!({"hosts": []})),
        Err(_) => json!({"hosts": []}),
    }
}

async fn save_hosts(data: &Value) -> Result<(), String> {
    let content = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    let tmp = format!("{}.tmp", HOSTS_FILE);
    tokio::fs::write(&tmp, &content)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(&tmp, HOSTS_FILE)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Migrate old servers.json + wol-schedules.json into hosts.json on first load.
pub async fn ensure_hosts_file() {
    if tokio::fs::metadata(HOSTS_FILE).await.is_ok() {
        return;
    }

    let servers = match tokio::fs::read_to_string("/data/servers.json").await {
        Ok(c) => serde_json::from_str::<Value>(&c)
            .ok()
            .and_then(|d| d.get("servers").cloned())
            .and_then(|s| s.as_array().cloned())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    let schedules = match tokio::fs::read_to_string("/data/wol-schedules.json").await {
        Ok(c) => serde_json::from_str::<Value>(&c)
            .ok()
            .and_then(|d| d.get("schedules").cloned())
            .and_then(|s| s.as_array().cloned())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    let mut hosts: Vec<Value> = Vec::new();

    for server in &servers {
        let id = server.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();

        // Collect schedules that belong to this server
        let host_schedules: Vec<Value> = schedules
            .iter()
            .filter(|s| s.get("serverId").and_then(|i| i.as_str()) == Some(&id))
            .cloned()
            .collect();

        let host = json!({
            "id": id,
            "name": server.get("name").and_then(|n| n.as_str()).unwrap_or(""),
            "host": server.get("host").and_then(|h| h.as_str()).unwrap_or(""),
            "port": server.get("port").and_then(|p| p.as_u64()).unwrap_or(22),
            "username": server.get("username").and_then(|u| u.as_str()).unwrap_or("root"),
            "interface": server.get("interface"),
            "mac": server.get("mac"),
            "groups": server.get("groups").cloned().unwrap_or(json!([])),
            "interfaces": server.get("interfaces").cloned().unwrap_or(json!([])),
            "status": server.get("status").and_then(|s| s.as_str()).unwrap_or("unknown"),
            "latency": server.get("latency").and_then(|l| l.as_u64()).unwrap_or(0),
            "lastSeen": server.get("lastSeen"),
            "schedules": host_schedules,
            "lxc": null,
            "createdAt": server.get("createdAt"),
            "updatedAt": server.get("updatedAt")
        });
        hosts.push(host);
    }

    let data = json!({"hosts": hosts});
    if let Err(e) = save_hosts(&data).await {
        tracing::error!("Failed to create hosts.json: {}", e);
    } else {
        tracing::info!("Migrated {} servers + {} schedules → hosts.json", servers.len(), schedules.len());
    }
}

// ── Host CRUD ────────────────────────────────────────────────────────────

async fn list_hosts(State(state): State<ApiState>) -> Json<Value> {
    let data = load_hosts().await;
    let mut hosts = data.get("hosts").cloned().unwrap_or(json!([]));

    // Override status based on live registry connections + power state
    if let Some(registry) = &state.registry {
        let conns = registry.host_connections.read().await;
        if let Some(arr) = hosts.as_array_mut() {
            for host in arr.iter_mut() {
                if let Some(id) = host.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()) {
                    let connected = conns.contains_key(id.as_str());
                    if !connected && host.get("status").and_then(|s| s.as_str()) == Some("online") {
                        host["status"] = json!("offline");
                    }
                    // Include current power state from registry
                    let power_state = registry.get_host_power_state(&id).await;
                    host["power_state"] = json!(power_state);
                }
            }
        }
    }

    Json(json!({"success": true, "hosts": hosts}))
}

async fn list_groups() -> Json<Value> {
    let data = load_hosts().await;
    let mut groups = std::collections::BTreeSet::new();
    if let Some(hosts) = data.get("hosts").and_then(|s| s.as_array()) {
        for host in hosts {
            if let Some(hg) = host.get("groups").and_then(|g| g.as_array()) {
                for g in hg {
                    if let Some(name) = g.as_str() {
                        groups.insert(name.to_string());
                    }
                }
            }
        }
    }
    let groups: Vec<String> = groups.into_iter().collect();
    Json(json!({"success": true, "groups": groups}))
}

async fn get_host(Path(id): Path<String>) -> Json<Value> {
    let data = load_hosts().await;
    if let Some(hosts) = data.get("hosts").and_then(|s| s.as_array()) {
        if let Some(host) = hosts.iter().find(|h| h.get("id").and_then(|i| i.as_str()) == Some(&id)) {
            return Json(json!({"success": true, "host": host}));
        }
    }
    Json(json!({"success": false, "error": "Hote non trouve"}))
}

#[derive(Deserialize)]
struct AddHostRequest {
    name: String,
    host: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default = "default_user")]
    username: String,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    interface: Option<String>,
    #[serde(default)]
    mac: Option<String>,
    #[serde(default)]
    groups: Vec<String>,
}

fn default_port() -> u16 { 22 }
fn default_user() -> String { "root".to_string() }

async fn add_host(Json(body): Json<AddHostRequest>) -> Json<Value> {
    if let Err(e) = ensure_ssh_key().await {
        return Json(json!({"success": false, "error": format!("SSH key error: {}", e)}));
    }

    if let Some(ref password) = body.password {
        if let Err(e) = setup_ssh_key(&body.host, body.port, &body.username, password).await {
            return Json(json!({"success": false, "error": format!("SSH setup failed: {}", e)}));
        }
    }

    let interfaces = get_remote_interfaces(&body.host, body.port, &body.username).await;
    let mac = body.mac.or_else(|| {
        interfaces.as_ref().ok().and_then(|ifaces| {
            ifaces.iter().find_map(|i| {
                let name = i.get("ifname").and_then(|n| n.as_str()).unwrap_or("");
                if body.interface.as_deref() == Some(name) || (body.interface.is_none() && name != "lo") {
                    i.get("address").and_then(|a| a.as_str()).map(String::from)
                } else {
                    None
                }
            })
        })
    });

    // Detect LAN interface: the interface whose IP matches the host address
    let detected_lan_interface = interfaces.as_ref().ok().and_then(|ifaces| {
        ifaces.iter().find_map(|i| {
            let ifname = i.get("ifname").and_then(|n| n.as_str())?;
            // Check addr_info array for matching IP
            if let Some(addr_info) = i.get("addr_info").and_then(|a| a.as_array()) {
                for addr in addr_info {
                    if addr.get("local").and_then(|l| l.as_str()) == Some(&body.host) {
                        return Some(ifname.to_string());
                    }
                }
            }
            None
        })
    });

    // Deploy hr-host-agent on the remote host
    if let Err(e) = deploy_host_agent(&body.host, body.port, &body.username, body.password.as_deref(), &body.name, detected_lan_interface.as_deref()).await {
        return Json(json!({"success": false, "error": format!("Agent deploy failed: {}", e)}));
    }
    tracing::info!("hr-host-agent deployed on {}", body.host);

    let id = uuid::Uuid::new_v4().to_string();
    let host = json!({
        "id": id,
        "name": body.name,
        "host": body.host,
        "port": body.port,
        "username": body.username,
        "interface": body.interface,
        "mac": mac,
        "groups": body.groups,
        "interfaces": interfaces.unwrap_or_default(),
        "status": "unknown",
        "latency": 0,
        "lastSeen": null,
        "lxc": null,
        "createdAt": chrono::Utc::now().to_rfc3339()
    });

    let mut data = load_hosts().await;
    let hosts = data.get_mut("hosts").and_then(|s| s.as_array_mut());
    match hosts {
        Some(arr) => arr.push(host.clone()),
        None => data["hosts"] = json!([host]),
    }

    if let Err(e) = save_hosts(&data).await {
        return Json(json!({"success": false, "error": e}));
    }

    Json(json!({"success": true, "host": host}))
}

async fn update_host(Path(id): Path<String>, Json(updates): Json<Value>) -> Json<Value> {
    let mut data = load_hosts().await;
    if let Some(hosts) = data.get_mut("hosts").and_then(|s| s.as_array_mut()) {
        if let Some(host) = hosts.iter_mut().find(|h| h.get("id").and_then(|i| i.as_str()) == Some(&id)) {
            if let Some(obj) = updates.as_object() {
                for (k, v) in obj {
                    if k != "id" && k != "schedules" {
                        host[k] = v.clone();
                    }
                }
            }
            host["updatedAt"] = json!(chrono::Utc::now().to_rfc3339());
        } else {
            return Json(json!({"success": false, "error": "Hote non trouve"}));
        }
    }

    if let Err(e) = save_hosts(&data).await {
        return Json(json!({"success": false, "error": e}));
    }
    Json(json!({"success": true}))
}

async fn delete_host(Path(id): Path<String>) -> Json<Value> {
    let mut data = load_hosts().await;
    if let Some(hosts) = data.get_mut("hosts").and_then(|s| s.as_array_mut()) {
        hosts.retain(|h| h.get("id").and_then(|i| i.as_str()) != Some(&id));
    }
    if let Err(e) = save_hosts(&data).await {
        return Json(json!({"success": false, "error": e}));
    }
    Json(json!({"success": true}))
}

// ── Connection & info ────────────────────────────────────────────────────

async fn test_connection(Path(id): Path<String>) -> Json<Value> {
    let data = load_hosts().await;
    let host = match find_host(&data, &id) {
        Some(h) => h,
        None => return Json(json!({"success": false, "error": "Hote non trouve"})),
    };

    let addr = host.get("host").and_then(|h| h.as_str()).unwrap_or("");
    let port = host.get("port").and_then(|p| p.as_u64()).unwrap_or(22) as u16;
    let user = host.get("username").and_then(|u| u.as_str()).unwrap_or("root");

    match ssh_command(addr, port, user, "echo ok").await {
        Ok(output) => Json(json!({"success": true, "output": output.trim()})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn get_host_info(Path(id): Path<String>) -> Json<Value> {
    let data = load_hosts().await;
    let host = match find_host(&data, &id) {
        Some(h) => h,
        None => return Json(json!({"success": false, "error": "Hote non trouve"})),
    };

    let addr = host.get("host").and_then(|h| h.as_str()).unwrap_or("");
    let port = host.get("port").and_then(|p| p.as_u64()).unwrap_or(22) as u16;
    let user = host.get("username").and_then(|u| u.as_str()).unwrap_or("root");

    let info_cmd = "hostname && uname -r && uptime -p && free -b | head -2 && df -B1 / | tail -1";
    match ssh_command(addr, port, user, info_cmd).await {
        Ok(output) => Json(json!({"success": true, "info": output})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

// ── Power actions ────────────────────────────────────────────────────────

async fn wake(Path(id): Path<String>, State(state): State<ApiState>) -> Json<Value> {
    // Use registry state machine if available
    if let Some(registry) = &state.registry {
        match registry.request_wake_host(&id).await {
            Ok(result) => {
                let action = match result {
                    hr_common::events::WakeResult::WolSent => "wol_sent",
                    hr_common::events::WakeResult::AlreadyWaking => "already_waking",
                    hr_common::events::WakeResult::AlreadyOnline => "already_online",
                };
                return Json(json!({"success": true, "action": action}));
            }
            Err(e) => return Json(json!({"success": false, "error": e})),
        }
    }

    // Fallback: direct WOL if no registry
    let data = load_hosts().await;
    let host = match find_host(&data, &id) {
        Some(h) => h,
        None => return Json(json!({"success": false, "error": "Hote non trouve"})),
    };
    let mac = match host.get("wol_mac").and_then(|m| m.as_str())
        .or_else(|| host.get("mac").and_then(|m| m.as_str())) {
        Some(m) => m,
        None => return Json(json!({"success": false, "error": "Adresse MAC non configuree"})),
    };
    match hr_registry::AgentRegistry::send_wol_packet(mac).await {
        Ok(()) => Json(json!({"success": true, "action": "wol_sent", "mac": mac})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn shutdown_host(Path(id): Path<String>, State(state): State<ApiState>) -> Json<Value> {
    // Check power state conflicts
    if let Some(registry) = &state.registry {
        if let Err(e) = registry.request_power_action(&id, hr_common::events::PowerAction::Shutdown).await {
            return Json(json!({"success": false, "error": e}));
        }
        // Try agent first
        if registry.send_host_command(
            &id,
            hr_registry::protocol::HostRegistryMessage::PowerOff,
        ).await.is_ok() {
            return Json(json!({"success": true, "action": "poweroff", "via": "agent"}));
        }
    }
    // SSH fallback
    let data = load_hosts().await;
    let host = match find_host(&data, &id) {
        Some(h) => h,
        None => return Json(json!({"success": false, "error": "Hote non trouve"})),
    };
    ssh_power_action(&host, "poweroff || shutdown -h now").await
}

async fn reboot_host(Path(id): Path<String>, State(state): State<ApiState>) -> Json<Value> {
    // Check power state conflicts
    if let Some(registry) = &state.registry {
        if let Err(e) = registry.request_power_action(&id, hr_common::events::PowerAction::Reboot).await {
            return Json(json!({"success": false, "error": e}));
        }
        if registry.send_host_command(
            &id,
            hr_registry::protocol::HostRegistryMessage::Reboot,
        ).await.is_ok() {
            return Json(json!({"success": true, "action": "reboot", "via": "agent"}));
        }
    }
    let data = load_hosts().await;
    let host = match find_host(&data, &id) {
        Some(h) => h,
        None => return Json(json!({"success": false, "error": "Hote non trouve"})),
    };
    ssh_power_action(&host, "reboot").await
}

#[derive(Deserialize)]
struct BulkRequest {
    #[serde(rename = "hostIds")]
    host_ids: Vec<String>,
}

async fn bulk_wake(State(state): State<ApiState>, Json(body): Json<BulkRequest>) -> Json<Value> {
    let mut results = Vec::new();
    for id in &body.host_ids {
        if let Some(registry) = &state.registry {
            match registry.request_wake_host(id).await {
                Ok(result) => {
                    let action = match result {
                        hr_common::events::WakeResult::WolSent => "wol_sent",
                        hr_common::events::WakeResult::AlreadyWaking => "already_waking",
                        hr_common::events::WakeResult::AlreadyOnline => "already_online",
                    };
                    results.push(json!({"id": id, "success": true, "action": action}));
                }
                Err(e) => results.push(json!({"id": id, "success": false, "error": e})),
            }
        } else {
            // Fallback: direct WOL
            let data = load_hosts().await;
            if let Some(host) = find_host(&data, id) {
                let mac = host.get("wol_mac").and_then(|m| m.as_str())
                    .or_else(|| host.get("mac").and_then(|m| m.as_str()));
                if let Some(mac) = mac {
                    let success = hr_registry::AgentRegistry::send_wol_packet(mac).await.is_ok();
                    results.push(json!({"id": id, "success": success}));
                } else {
                    results.push(json!({"id": id, "success": false, "error": "No MAC"}));
                }
            } else {
                results.push(json!({"id": id, "success": false, "error": "Not found"}));
            }
        }
    }
    Json(json!({"success": true, "results": results}))
}

async fn bulk_shutdown(Json(body): Json<BulkRequest>) -> Json<Value> {
    let data = load_hosts().await;
    let mut results = Vec::new();
    for id in &body.host_ids {
        if let Some(host) = find_host(&data, id) {
            let result = ssh_power_action(&host, "poweroff || shutdown -h now").await;
            results.push(json!({"id": id, "result": result.0}));
        } else {
            results.push(json!({"id": id, "success": false, "error": "Not found"}));
        }
    }
    Json(json!({"success": true, "results": results}))
}

async fn sleep_host(Path(id): Path<String>, State(state): State<ApiState>) -> Json<Value> {
    // Check power state conflicts
    if let Some(registry) = &state.registry {
        if let Err(e) = registry.request_power_action(&id, hr_common::events::PowerAction::Suspend).await {
            return Json(json!({"success": false, "error": e}));
        }
        if registry.send_host_command(
            &id,
            hr_registry::protocol::HostRegistryMessage::SuspendHost,
        ).await.is_ok() {
            return Json(json!({"success": true, "action": "sleep", "via": "agent"}));
        }
    }
    let data = load_hosts().await;
    let host = match find_host(&data, &id) {
        Some(h) => h,
        None => return Json(json!({"success": false, "error": "Hote non trouve"})),
    };
    ssh_power_action(&host, "systemctl suspend").await
}

#[derive(Deserialize)]
struct SetWolMacRequest {
    mac: String,
}

#[derive(Deserialize)]
struct SetAutoOffRequest {
    /// "sleep", "shutdown", or "off"
    mode: String,
    #[serde(default)]
    minutes: u32,
}

async fn set_wol_mac(Path(id): Path<String>, State(state): State<ApiState>, Json(body): Json<SetWolMacRequest>) -> Json<Value> {
    let mut data = load_hosts().await;
    if let Some(host) = find_host_mut(&mut data, &id) {
        host["wol_mac"] = json!(body.mac);
        host["updatedAt"] = json!(chrono::Utc::now().to_rfc3339());
    } else {
        return Json(json!({"success": false, "error": "Hote non trouve"}));
    }
    if let Err(e) = save_hosts(&data).await {
        return Json(json!({"success": false, "error": e}));
    }
    // Invalidate cached MAC in power state machine
    if let Some(registry) = &state.registry {
        registry.invalidate_host_mac_cache(&id).await;
    }
    Json(json!({"success": true}))
}

async fn set_auto_off(
    Path(id): Path<String>,
    State(state): State<ApiState>,
    Json(body): Json<SetAutoOffRequest>,
) -> Json<Value> {
    let mut data = load_hosts().await;
    if let Some(host) = find_host_mut(&mut data, &id) {
        host["auto_off_mode"] = json!(body.mode);
        host["auto_off_minutes"] = json!(body.minutes);
        // Clean up old field if present
        if let Some(obj) = host.as_object_mut() {
            obj.remove("sleep_timeout_minutes");
        }
        host["updatedAt"] = json!(chrono::Utc::now().to_rfc3339());
    } else {
        return Json(json!({"success": false, "error": "Hote non trouve"}));
    }
    if let Err(e) = save_hosts(&data).await {
        return Json(json!({"success": false, "error": e}));
    }

    // Push to connected agent
    if body.mode != "off" && body.minutes > 0 {
        if let Some(registry) = &state.registry {
            let mode = match body.mode.as_str() {
                "shutdown" => hr_registry::protocol::AutoOffMode::Shutdown,
                _ => hr_registry::protocol::AutoOffMode::Sleep,
            };
            let _ = registry.send_host_command(
                &id,
                hr_registry::protocol::HostRegistryMessage::SetAutoOff {
                    mode,
                    minutes: body.minutes,
                },
            ).await;
        }
    } else if let Some(registry) = &state.registry {
        // Send minutes=0 to disable auto-off on agent
        let _ = registry.send_host_command(
            &id,
            hr_registry::protocol::HostRegistryMessage::SetAutoOff {
                mode: hr_registry::protocol::AutoOffMode::Sleep,
                minutes: 0,
            },
        ).await;
    }
    Json(json!({"success": true}))
}

async fn get_host_metrics(Path(id): Path<String>, State(state): State<ApiState>) -> Json<Value> {
    let registry = match &state.registry {
        Some(r) => r,
        None => return Json(json!({"success": false, "error": "No registry"})),
    };
    let conns = registry.host_connections.read().await;
    if let Some(conn) = conns.get(&id) {
        if let Some(ref metrics) = conn.metrics {
            return Json(json!({
                "success": true,
                "metrics": {
                    "cpuPercent": metrics.cpu_percent,
                    "memoryUsedBytes": metrics.memory_used_bytes,
                    "memoryTotalBytes": metrics.memory_total_bytes,
                    "diskUsedBytes": metrics.disk_used_bytes,
                    "diskTotalBytes": metrics.disk_total_bytes,
                    "loadAvg": metrics.load_avg,
                }
            }));
        }
    }
    Json(json!({"success": false, "error": "No metrics available"}))
}

async fn update_host_agents(State(state): State<ApiState>) -> Json<Value> {
    let registry = match &state.registry {
        Some(r) => r,
        None => return Json(json!({"success": false, "error": "No registry"})),
    };

    let binary_path = std::path::Path::new(HOST_AGENT_BINARY);
    if !binary_path.exists() {
        return Json(json!({"success": false, "error": "Host agent binary not found"}));
    }

    use std::io::Read;
    let mut file = match std::fs::File::open(binary_path) {
        Ok(f) => f,
        Err(e) => return Json(json!({"success": false, "error": format!("Open binary: {}", e)})),
    };
    use ring::digest::{Context, SHA256};
    let mut ctx = Context::new(&SHA256);
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).unwrap_or(0);
        if n == 0 { break; }
        ctx.update(&buf[..n]);
    }
    let sha256 = hex::encode(ctx.finish().as_ref());

    let version = std::fs::metadata(binary_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.format("%Y%m%d-%H%M%S").to_string()
        })
        .unwrap_or_else(|| "unknown".to_string());

    let download_url = format!("http://{}:{}/api/hosts/agents/binary", HOMEROUTE_LAN_IP, API_PORT);

    let conns = registry.host_connections.read().await;
    let mut notified = 0u32;
    for (_host_id, conn) in conns.iter() {
        let msg = hr_registry::protocol::HostRegistryMessage::PushAgentUpdate {
            version: version.clone(),
            download_url: download_url.clone(),
            sha256: sha256.clone(),
        };
        if conn.tx.send(msg).await.is_ok() {
            notified += 1;
        }
    }

    Json(json!({"success": true, "notified": notified, "version": version, "sha256": sha256}))
}

async fn serve_host_agent_binary() -> impl IntoResponse {
    match tokio::fs::read(HOST_AGENT_BINARY).await {
        Ok(data) => (
            axum::http::StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            data,
        ).into_response(),
        Err(_) => (axum::http::StatusCode::NOT_FOUND, "Binary not found").into_response(),
    }
}

// ── Remote container management ──────────────────────────────────────────

async fn start_container(
    Path((id, name)): Path<(String, String)>,
    State(state): State<ApiState>,
) -> Json<Value> {
    let registry = match &state.registry {
        Some(r) => r,
        None => return Json(json!({"success": false, "error": "No registry"})),
    };
    let container_name = if name.starts_with("hr-") { name } else { format!("hr-{name}") };
    match registry.send_host_command(
        &id,
        hr_registry::protocol::HostRegistryMessage::StartContainer { container_name: container_name.clone() },
    ).await {
        Ok(_) => Json(json!({"success": true, "message": format!("Start command sent for {container_name}")})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn stop_container(
    Path((id, name)): Path<(String, String)>,
    State(state): State<ApiState>,
) -> Json<Value> {
    let registry = match &state.registry {
        Some(r) => r,
        None => return Json(json!({"success": false, "error": "No registry"})),
    };
    let container_name = if name.starts_with("hr-") { name } else { format!("hr-{name}") };
    match registry.send_host_command(
        &id,
        hr_registry::protocol::HostRegistryMessage::StopContainer { container_name: container_name.clone() },
    ).await {
        Ok(_) => Json(json!({"success": true, "message": format!("Stop command sent for {container_name}")})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn delete_container(
    Path((id, name)): Path<(String, String)>,
    State(state): State<ApiState>,
) -> Json<Value> {
    let registry = match &state.registry {
        Some(r) => r,
        None => return Json(json!({"success": false, "error": "No registry"})),
    };
    let container_name = if name.starts_with("hr-") { name } else { format!("hr-{name}") };
    match registry.send_host_command(
        &id,
        hr_registry::protocol::HostRegistryMessage::DeleteContainer { container_name: container_name.clone() },
    ).await {
        Ok(_) => Json(json!({"success": true, "message": format!("Delete command sent for {container_name}")})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

#[derive(Deserialize)]
struct ExecRequest {
    container_name: String,
    command: Vec<String>,
}

async fn exec_on_host(
    Path(id): Path<String>,
    State(state): State<ApiState>,
    Json(body): Json<ExecRequest>,
) -> Json<Value> {
    let registry = match &state.registry {
        Some(r) => r,
        None => return Json(json!({"success": false, "error": "No registry"})),
    };
    match registry.exec_in_remote_container(&id, &body.container_name, body.command).await {
        Ok((success, stdout, stderr)) => Json(json!({
            "success": success,
            "stdout": stdout,
            "stderr": stderr,
        })),
        Err(e) => Json(json!({"success": false, "error": format!("{e}")})),
    }
}

// ── Host-agent WebSocket ─────────────────────────────────────────────────

async fn host_agent_ws(
    ws: WebSocketUpgrade,
    State(state): State<ApiState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_host_agent_socket(socket, state))
}

async fn handle_host_agent_socket(mut socket: WebSocket, state: ApiState) {
    use hr_registry::protocol::{HostAgentMessage, HostRegistryMessage};
    use hr_common::events::HostStatusEvent;

    let registry = match &state.registry {
        Some(r) => r.clone(),
        None => {
            tracing::warn!("Host agent WS: no registry available");
            return;
        }
    };

    // Wait for Auth message (5s timeout)
    let auth_msg = tokio::time::timeout(std::time::Duration::from_secs(5), socket.recv()).await;
    let (host_id, host_name, version) = match auth_msg {
        Ok(Some(Ok(Message::Text(text)))) => {
            match serde_json::from_str::<HostAgentMessage>(&text) {
                Ok(HostAgentMessage::Auth { token: _, host_name, version }) => {
                    let data = load_hosts().await;
                    let host_id = data
                        .get("hosts")
                        .and_then(|h| h.as_array())
                        .and_then(|hosts| {
                            hosts.iter().find(|h| {
                                h.get("name").and_then(|n| n.as_str()) == Some(&host_name)
                            })
                        })
                        .and_then(|h| h.get("id").and_then(|i| i.as_str()))
                        .map(|s| s.to_string());

                    match host_id {
                        Some(id) => (id, host_name, version),
                        None => {
                            tracing::warn!("Host agent auth failed: unknown host '{}'", host_name);
                            let _ = socket.send(Message::Text(
                                serde_json::to_string(&HostRegistryMessage::AuthResult {
                                    success: false,
                                    error: Some("Unknown host".to_string()),
                                }).unwrap().into()
                            )).await;
                            return;
                        }
                    }
                }
                _ => {
                    tracing::warn!("Host agent: expected Auth message");
                    return;
                }
            }
        }
        _ => {
            tracing::warn!("Host agent: auth timeout or error");
            return;
        }
    };

    // Send auth success
    if socket.send(Message::Text(
        serde_json::to_string(&HostRegistryMessage::AuthResult {
            success: true,
            error: None,
        }).unwrap().into()
    )).await.is_err() {
        return;
    }

    tracing::info!("Host agent authenticated: {} ({})", host_name, host_id);

    // Register connection
    let (tx, mut rx) = mpsc::channel::<HostRegistryMessage>(32);
    registry.on_host_connected(host_id.clone(), host_name.clone(), tx, version).await;

    // Mark host online
    update_host_status(&host_id, "online", &state.events.host_status).await;

    // Push auto-off config to agent on connect
    {
        let data = load_hosts().await;
        if let Some(host) = find_host(&data, &host_id) {
            let mode_str = host.get("auto_off_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("off");
            let minutes = host.get("auto_off_minutes")
                .and_then(|v| v.as_u64())
                // Backward compat: fallback to old field
                .or_else(|| host.get("sleep_timeout_minutes").and_then(|v| v.as_u64()))
                .unwrap_or(0) as u32;
            if mode_str != "off" && minutes > 0 {
                let mode = match mode_str {
                    "shutdown" => hr_registry::protocol::AutoOffMode::Shutdown,
                    _ => hr_registry::protocol::AutoOffMode::Sleep,
                };
                let _ = registry.send_host_command(
                    &host_id,
                    hr_registry::protocol::HostRegistryMessage::SetAutoOff { mode, minutes },
                ).await;
            }
        }
    }

    // Track active transfers for remote→local migration (transfer_id → file + container_name)
    let mut active_transfers: std::collections::HashMap<String, (tokio::fs::File, String)> = std::collections::HashMap::new();

    // Heartbeat timeout: agent sends every 5s, detect offline within 10s
    let heartbeat_timeout = std::time::Duration::from_secs(10);
    let timeout_sleep = tokio::time::sleep(heartbeat_timeout);
    tokio::pin!(timeout_sleep);

    // Bidirectional message loop
    loop {
        tokio::select! {
            // Messages from registry → host-agent
            Some(msg) = rx.recv() => {
                let text = match serde_json::to_string(&msg) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            // Heartbeat timeout — host likely asleep or unreachable
            _ = &mut timeout_sleep => {
                tracing::warn!("Host agent heartbeat timeout: {} ({})", host_name, host_id);
                break;
            }
            // Messages from host-agent → registry
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Any message from the agent resets the heartbeat deadline
                        timeout_sleep.as_mut().reset(tokio::time::Instant::now() + heartbeat_timeout);
                        if let Ok(agent_msg) = serde_json::from_str::<HostAgentMessage>(&text) {
                            match agent_msg {
                                HostAgentMessage::Heartbeat { .. } => {
                                    registry.update_host_heartbeat(&host_id).await;
                                    update_host_last_seen(&host_id).await;
                                }
                                HostAgentMessage::Metrics(metrics) => {
                                    registry.update_host_metrics(&host_id, metrics.clone()).await;
                                    let _ = state.events.host_metrics.send(hr_common::events::HostMetricsEvent {
                                        host_id: host_id.clone(),
                                        cpu_percent: metrics.cpu_percent,
                                        memory_used_bytes: metrics.memory_used_bytes,
                                        memory_total_bytes: metrics.memory_total_bytes,
                                    });
                                }
                                HostAgentMessage::NetworkInterfaces(interfaces) => {
                                    registry.update_host_interfaces(&host_id, interfaces.clone()).await;
                                    // Persist to hosts.json
                                    let mut data = load_hosts().await;
                                    if let Some(host) = find_host_mut(&mut data, &host_id) {
                                        let ifaces: Vec<serde_json::Value> = interfaces.iter().map(|i| json!({
                                            "ifname": i.name,
                                            "address": i.mac,
                                            "ipv4": i.ipv4,
                                            "is_up": i.is_up,
                                        })).collect();
                                        host["interfaces"] = json!(ifaces);
                                        let _ = save_hosts(&data).await;
                                    }
                                }
                                HostAgentMessage::ContainerList(containers) => {
                                    registry.update_host_containers(&host_id, containers).await;
                                }
                                HostAgentMessage::ImportComplete { transfer_id, container_name } => {
                                    tracing::info!(transfer_id = %transfer_id, container = %container_name, "Host import complete");
                                    registry.on_host_import_complete(&host_id, &transfer_id, &container_name).await;
                                }
                                HostAgentMessage::ImportFailed { transfer_id, error } => {
                                    tracing::error!(transfer_id = %transfer_id, %error, "Host import failed");
                                    registry.on_host_import_failed(&host_id, &transfer_id, &error).await;
                                }
                                HostAgentMessage::ExecResult { request_id, success, stdout, stderr } => {
                                    tracing::info!(request_id = %request_id, success, "Host exec result");
                                    registry.on_host_exec_result(&host_id, &request_id, success, &stdout, &stderr).await;
                                }
                                HostAgentMessage::ExportReady { transfer_id, container_name, size_bytes } => {
                                    // Use container_name from message, or fall back to registry lookup
                                    let cname = if container_name.is_empty() {
                                        registry.take_transfer_container_name(&transfer_id).await.unwrap_or_default()
                                    } else {
                                        container_name
                                    };
                                    if cname.is_empty() {
                                        tracing::error!(transfer_id = %transfer_id, "ExportReady with unknown container_name");
                                        registry.on_host_import_failed(&host_id, &transfer_id, "Unknown container name for transfer").await;
                                        continue;
                                    }
                                    tracing::info!(transfer_id = %transfer_id, container = %cname, size_bytes, "Host export ready, creating local transfer file");
                                    let path = format!("/tmp/{}.tar.gz", transfer_id);
                                    match tokio::fs::File::create(&path).await {
                                        Ok(file) => {
                                            active_transfers.insert(transfer_id.clone(), (file, cname));
                                        }
                                        Err(e) => {
                                            tracing::error!(transfer_id = %transfer_id, %e, "Failed to create transfer file");
                                            registry.on_host_import_failed(&host_id, &transfer_id, &format!("Failed to create local file: {e}")).await;
                                        }
                                    }
                                }
                                HostAgentMessage::ExportFailed { transfer_id, error } => {
                                    tracing::error!(transfer_id = %transfer_id, %error, "Host export failed");
                                    active_transfers.remove(&transfer_id);
                                    let _ = tokio::fs::remove_file(format!("/tmp/{}.tar.gz", transfer_id)).await;
                                    registry.on_host_export_failed(&host_id, &transfer_id, &error).await;
                                }
                                HostAgentMessage::TransferChunk { transfer_id, data } => {
                                    use base64::Engine;
                                    use tokio::io::AsyncWriteExt;
                                    if let Some((file, _)) = active_transfers.get_mut(&transfer_id) {
                                        match base64::engine::general_purpose::STANDARD.decode(&data) {
                                            Ok(bytes) => {
                                                if let Err(e) = file.write_all(&bytes).await {
                                                    tracing::error!(transfer_id = %transfer_id, %e, "Failed to write chunk");
                                                    active_transfers.remove(&transfer_id);
                                                    let _ = tokio::fs::remove_file(format!("/tmp/{}.tar.gz", transfer_id)).await;
                                                    registry.on_host_import_failed(&host_id, &transfer_id, &format!("Write error: {e}")).await;
                                                }
                                            }
                                            Err(e) => {
                                                tracing::error!(transfer_id = %transfer_id, %e, "Failed to decode chunk");
                                                active_transfers.remove(&transfer_id);
                                                let _ = tokio::fs::remove_file(format!("/tmp/{}.tar.gz", transfer_id)).await;
                                                registry.on_host_import_failed(&host_id, &transfer_id, &format!("Base64 decode error: {e}")).await;
                                            }
                                        }
                                    }
                                }
                                HostAgentMessage::TransferComplete { transfer_id } => {
                                    tracing::info!(transfer_id = %transfer_id, "Host transfer complete, starting local import");
                                    if let Some((file, cname)) = active_transfers.remove(&transfer_id) {
                                        // Close the file handle
                                        drop(file);
                                        // Spawn import task so we don't block the WS loop
                                        let tid = transfer_id.clone();
                                        let reg = registry.clone();
                                        let hid = host_id.clone();
                                        tokio::spawn(async move {
                                            handle_local_import(reg, hid, tid, cname).await;
                                        });
                                    } else {
                                        tracing::warn!(transfer_id = %transfer_id, "TransferComplete for unknown transfer");
                                    }
                                }
                                HostAgentMessage::AutoOffNotify { mode } => {
                                    let action = match mode {
                                        hr_registry::protocol::AutoOffMode::Sleep => {
                                            tracing::info!("Host agent auto-sleep: {} ({})", host_name, host_id);
                                            hr_common::events::PowerAction::Suspend
                                        }
                                        hr_registry::protocol::AutoOffMode::Shutdown => {
                                            tracing::info!("Host agent auto-shutdown: {} ({})", host_name, host_id);
                                            hr_common::events::PowerAction::Shutdown
                                        }
                                    };
                                    let _ = registry.request_power_action(&host_id, action).await;
                                }
                                HostAgentMessage::Auth { .. } => {}
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    // Clean up any pending transfers
    for (tid, _) in active_transfers {
        let _ = tokio::fs::remove_file(format!("/tmp/{}.tar.gz", tid)).await;
    }

    // Mark host offline
    update_host_status(&host_id, "offline", &state.events.host_status).await;

    registry.on_host_disconnected(&host_id).await;
    tracing::info!("Host agent disconnected: {} ({})", host_name, host_id);
}

// ── Local import for remote→local migration ─────────────────────────────

async fn lxc_cmd(args: &[&str], timeout_secs: u64) -> Result<std::process::Output, String> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        tokio::process::Command::new("lxc").args(args).output(),
    ).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(format!("lxc error: {e}")),
        Err(_) => Err(format!("lxc {} timed out after {timeout_secs}s", args.first().unwrap_or(&""))),
    }
}

/// Import a container locally after receiving all chunks from a remote host-agent.
/// This mirrors the host-agent's `handle_import()` but runs on the master.
async fn handle_local_import(
    registry: std::sync::Arc<hr_registry::AgentRegistry>,
    source_host_id: String,
    transfer_id: String,
    container_name: String,
) {
    let import_path = format!("/tmp/{}.tar.gz", transfer_id);

    // Verify the file exists and is non-empty
    match tokio::fs::metadata(&import_path).await {
        Ok(m) if m.len() == 0 => {
            tracing::error!(transfer_id = %transfer_id, "Transfer file is empty");
            registry.on_host_import_failed(&source_host_id, &transfer_id, "Transfer file is empty").await;
            let _ = tokio::fs::remove_file(&import_path).await;
            return;
        }
        Err(e) => {
            tracing::error!(transfer_id = %transfer_id, %e, "Transfer file missing");
            registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("Transfer file missing: {e}")).await;
            return;
        }
        Ok(m) => {
            tracing::info!(transfer_id = %transfer_id, size_bytes = m.len(), "Starting local LXC import");
        }
    }

    // Clean up stale container on target (e.g., from a previous failed migration)
    let _ = lxc_cmd(&["delete", &container_name, "--force"], 30).await;

    // Ensure workspace volume exists before import (container config references it as a device)
    let vol_name = format!("{container_name}-workspace");
    let vol_check = lxc_cmd(&["storage", "volume", "show", "default", &vol_name], 30).await;
    if !vol_check.map(|o| o.status.success()).unwrap_or(false) {
        tracing::info!(volume = %vol_name, "Creating workspace volume before import");
        let _ = lxc_cmd(&["storage", "volume", "create", "default", &vol_name], 30).await;
    }

    // Import the container
    let import = lxc_cmd(&["import", &import_path], 300).await;

    match import {
        Ok(output) if output.status.success() => {
            tracing::info!(transfer_id = %transfer_id, container = %container_name, "LXC import successful");

            // Check if workspace volume was restored by import
            let vol_name = format!("{container_name}-workspace");
            let vol_check = lxc_cmd(&["storage", "volume", "show", "default", &vol_name], 30).await;
            let vol_exists = vol_check.map(|o| o.status.success()).unwrap_or(false);
            if !vol_exists {
                tracing::info!(volume = %vol_name, "Workspace volume not in export, creating fresh");
                let _ = lxc_cmd(&["storage", "volume", "create", "default", &vol_name], 30).await;
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!(transfer_id = %transfer_id, %stderr, "LXC import failed");
            registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("lxc import failed: {stderr}")).await;
            let _ = tokio::fs::remove_file(&import_path).await;
            return;
        }
        Err(e) => {
            tracing::error!(transfer_id = %transfer_id, %e, "LXC import command error");
            registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("Import command error: {e}")).await;
            let _ = tokio::fs::remove_file(&import_path).await;
            return;
        }
    }

    // Assign profile and start
    tracing::info!(transfer_id = %transfer_id, container = %container_name, "Container imported, assigning profile and starting");

    // Assign the homeroute-agent profile (like the host-agent does)
    let profile_assign = lxc_cmd(&["profile", "assign", &container_name, "default,homeroute-agent"], 30).await;
    match profile_assign {
        Ok(output) if output.status.success() => {
            tracing::info!(container = %container_name, "Profile assigned successfully");
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!(container = %container_name, %stderr, "Profile assignment failed");
            registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("Profile assignment failed: {stderr}")).await;
            let _ = tokio::fs::remove_file(&import_path).await;
            return;
        }
        Err(e) => {
            tracing::error!(container = %container_name, %e, "Profile assignment command failed");
            registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("Profile command error: {e}")).await;
            let _ = tokio::fs::remove_file(&import_path).await;
            return;
        }
    }

    // Start the container
    let start = lxc_cmd(&["start", &container_name], 60).await;

    match start {
        Ok(output) if output.status.success() => {
            tracing::info!(transfer_id = %transfer_id, container = %container_name, "Container started successfully");
            // Signal migration success — this unblocks the migration task in applications.rs
            registry.on_host_import_complete("local", &transfer_id, &container_name).await;
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!(transfer_id = %transfer_id, %stderr, "Container imported but start failed");
            registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("Start failed: {stderr}")).await;
        }
        Err(e) => {
            tracing::error!(transfer_id = %transfer_id, %e, "Start command error");
            registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("Start command error: {e}")).await;
        }
    }

    // Cleanup transfer file
    let _ = tokio::fs::remove_file(&import_path).await;
}

// ── Agent status helpers ─────────────────────────────────────────────────

async fn update_host_status(
    host_id: &str,
    status: &str,
    host_events: &tokio::sync::broadcast::Sender<hr_common::events::HostStatusEvent>,
) {
    let mut data = load_hosts().await;
    if let Some(host) = find_host_mut(&mut data, host_id) {
        let now = chrono::Utc::now().to_rfc3339();
        host["status"] = json!(status);
        host["lastSeen"] = json!(&now);
        let _ = save_hosts(&data).await;
    }
    let _ = host_events.send(hr_common::events::HostStatusEvent {
        host_id: host_id.to_string(),
        status: status.to_string(),
        latency_ms: None,
    });
}

async fn update_host_last_seen(host_id: &str) {
    let mut data = load_hosts().await;
    if let Some(host) = find_host_mut(&mut data, host_id) {
        host["lastSeen"] = json!(chrono::Utc::now().to_rfc3339());
        let _ = save_hosts(&data).await;
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn find_host<'a>(data: &'a Value, id: &str) -> Option<&'a Value> {
    data.get("hosts")?
        .as_array()?
        .iter()
        .find(|h| h.get("id").and_then(|i| i.as_str()) == Some(id))
}

fn find_host_mut<'a>(data: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    data.get_mut("hosts")?
        .as_array_mut()?
        .iter_mut()
        .find(|h| h.get("id").and_then(|i| i.as_str()) == Some(id))
}

async fn ssh_power_action(host: &Value, command: &str) -> Json<Value> {
    let addr = host.get("host").and_then(|h| h.as_str()).unwrap_or("");
    let port = host.get("port").and_then(|p| p.as_u64()).unwrap_or(22);
    let user = host.get("username").and_then(|u| u.as_str()).unwrap_or("root");

    let output = tokio::process::Command::new("ssh")
        .args([
            "-i", SSH_KEY_PATH,
            "-o", "StrictHostKeyChecking=no",
            "-o", "ConnectTimeout=15",
            "-o", "BatchMode=yes",
            "-p", &port.to_string(),
            &format!("{}@{}", user, addr),
            &format!("sudo {}", command),
        ])
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() || o.status.code() == Some(255) => {
            Json(json!({"success": true, "action": command.split_whitespace().next().unwrap_or(command)}))
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            Json(json!({"success": false, "error": format!("SSH error: {}", stderr)}))
        }
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

// ── SSH helpers ──────────────────────────────────────────────────────────

async fn ensure_ssh_key() -> Result<(), String> {
    if tokio::fs::metadata(SSH_KEY_PATH).await.is_ok() {
        return Ok(());
    }

    let _ = tokio::fs::create_dir_all("/data/ssh").await;

    let output = tokio::process::Command::new("ssh-keygen")
        .args(["-t", "rsa", "-b", "4096", "-f", SSH_KEY_PATH, "-N", ""])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    let _ = tokio::fs::set_permissions(
        SSH_KEY_PATH,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
    ).await;

    Ok(())
}

async fn setup_ssh_key(host: &str, port: u16, user: &str, password: &str) -> Result<(), String> {
    let pub_key = tokio::fs::read_to_string(SSH_PUB_KEY_PATH)
        .await
        .map_err(|e| format!("Read pub key: {}", e))?;

    let output = tokio::process::Command::new("sshpass")
        .args([
            "-p", password,
            "ssh",
            "-o", "StrictHostKeyChecking=no",
            "-o", "ConnectTimeout=15",
            "-p", &port.to_string(),
            &format!("{}@{}", user, host),
            &format!(
                "mkdir -p ~/.ssh && chmod 700 ~/.ssh && echo '{}' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys",
                pub_key.trim()
            ),
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    Ok(())
}

async fn ssh_command(host: &str, port: u16, user: &str, command: &str) -> Result<String, String> {
    let output = tokio::process::Command::new("ssh")
        .args([
            "-i", SSH_KEY_PATH,
            "-o", "StrictHostKeyChecking=no",
            "-o", "ConnectTimeout=15",
            "-o", "BatchMode=yes",
            "-p", &port.to_string(),
            &format!("{}@{}", user, host),
            command,
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("SSH failed: {}", stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn get_remote_interfaces(
    host: &str,
    port: u16,
    user: &str,
) -> Result<Vec<Value>, String> {
    let output = ssh_command(host, port, user, "ip -j addr show 2>/dev/null || ip addr show").await?;

    if let Ok(ifaces) = serde_json::from_str::<Vec<Value>>(&output) {
        return Ok(ifaces);
    }

    let mut interfaces = Vec::new();
    let mut current: Option<Value> = None;

    for line in output.lines() {
        if !line.starts_with(' ') && line.contains(':') {
            if let Some(iface) = current.take() {
                interfaces.push(iface);
            }
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 2 {
                current = Some(json!({"ifname": parts[1].trim()}));
            }
        } else if let Some(ref mut iface) = current {
            let line = line.trim();
            if line.starts_with("link/ether") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(mac) = parts.get(1) {
                    iface["address"] = json!(mac);
                }
            }
        }
    }
    if let Some(iface) = current {
        interfaces.push(iface);
    }

    Ok(interfaces)
}

// ── Host-agent deployment ────────────────────────────────────────────────

async fn deploy_host_agent(host: &str, port: u16, user: &str, password: Option<&str>, host_name: &str, lan_interface: Option<&str>) -> Result<(), String> {
    if tokio::fs::metadata(HOST_AGENT_BINARY).await.is_err() {
        return Err("hr-host-agent binary not found".to_string());
    }

    let password = password.ok_or("Password required for agent deployment")?;

    // 1. SCP binary to /tmp/
    let scp_output = tokio::process::Command::new("scp")
        .args([
            "-i", SSH_KEY_PATH,
            "-o", "StrictHostKeyChecking=no",
            "-o", "ConnectTimeout=15",
            "-P", &port.to_string(),
            HOST_AGENT_BINARY,
            &format!("{}@{}:/tmp/hr-host-agent", user, host),
        ])
        .output()
        .await
        .map_err(|e| format!("SCP failed: {}", e))?;

    if !scp_output.status.success() {
        let stderr = String::from_utf8_lossy(&scp_output.stderr);
        return Err(format!("SCP failed: {}", stderr));
    }

    // 2. Install via sshpass + sudo -S (password piped to stdin)
    let lan_line = match lan_interface {
        Some(iface) => format!("lan_interface = \"{}\"\n", iface),
        None => String::new(),
    };
    let config = format!(
        r#"homeroute_url = "{HOMEROUTE_LAN_IP}:{API_PORT}"
token = ""
host_name = "{host_name}"
{lan_line}"#,
    );

    let service_unit = r#"[Unit]
Description=HomeRoute Host Agent
After=network.target

[Service]
ExecStart=/usr/local/bin/hr-host-agent
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
"#;

    // Use a single sudo -S bash -c to run all commands with one password prompt
    let inner_cmds = format!(
        r#"mv /tmp/hr-host-agent /usr/local/bin/hr-host-agent && \
chmod +x /usr/local/bin/hr-host-agent && \
mkdir -p /etc/hr-host-agent && \
cat > /etc/hr-host-agent/config.toml << 'CONF'
{config}CONF
cat > /etc/systemd/system/hr-host-agent.service << 'SVC'
{service_unit}SVC
systemctl daemon-reload && \
systemctl enable --now hr-host-agent"#,
    );

    // Escape single quotes in inner_cmds for shell wrapping
    let escaped = inner_cmds.replace('\'', "'\\''");

    let setup_cmd = format!("echo '{password}' | sudo -S bash -c '{escaped}'");

    ssh_command(host, port, user, &setup_cmd).await?;

    Ok(())
}
