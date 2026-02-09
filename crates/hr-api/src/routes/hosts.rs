use axum::{
    extract::{Path, State, WebSocketUpgrade, ws::{Message, WebSocket}},
    response::IntoResponse,
    routing::{get, post, put},
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
        // Local host routes (must be before /{id} to avoid path conflicts)
        .route("/local/interfaces", get(get_local_interfaces_handler))
        .route("/local/config", put(update_local_config))
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
                    // Include latest metrics from live connection
                    if let Some(conn) = conns.get(id.as_str()) {
                        if let Some(ref m) = conn.metrics {
                            host["metrics"] = json!({
                                "cpuPercent": m.cpu_percent,
                                "memoryUsedBytes": m.memory_used_bytes,
                                "memoryTotalBytes": m.memory_total_bytes,
                            });
                        }
                    }
                }
            }
        }
    }

    // Prepend synthetic "HomeRoute" local host entry
    let local_host = {
        let (lan_interface, container_storage_path) = if let Some(cm) = &state.container_manager {
            let cfg = cm.get_config().await;
            (cfg.lan_interface.clone(), cfg.container_storage_path.clone())
        } else {
            (None, "/var/lib/machines".to_string())
        };
        let interfaces = get_local_interfaces().await.unwrap_or_default();
        let local_metrics = get_local_metrics().await;
        json!({
            "id": "local",
            "name": "HomeRoute",
            "is_local": true,
            "host": "127.0.0.1",
            "port": 4000,
            "status": "online",
            "lan_interface": lan_interface,
            "container_storage_path": container_storage_path,
            "interfaces": interfaces,
            "metrics": local_metrics,
        })
    };

    let mut result = vec![local_host];
    if let Some(arr) = hosts.as_array() {
        result.extend(arr.iter().cloned());
    }

    Json(json!({"success": true, "hosts": result}))
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

// ── Local host helpers ────────────────────────────────────────────────────

async fn get_local_interfaces() -> Result<Vec<Value>, String> {
    let output = tokio::process::Command::new("ip")
        .args(["-j", "addr", "show"])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw: Vec<Value> = serde_json::from_str(&stdout).map_err(|e| e.to_string())?;

    // Normalize to match the format stored in hosts.json: { ifname, address, ipv4, is_up }
    let normalized: Vec<Value> = raw.iter().filter_map(|iface| {
        let ifname = iface.get("ifname")?.as_str()?;
        let address = iface.get("address").and_then(|a| a.as_str()).unwrap_or("");
        let operstate = iface.get("operstate").and_then(|o| o.as_str()).unwrap_or("unknown");
        let ipv4 = iface.get("addr_info")
            .and_then(|a| a.as_array())
            .and_then(|addrs| {
                addrs.iter().find_map(|a| {
                    if a.get("family").and_then(|f| f.as_str()) == Some("inet") {
                        a.get("local").and_then(|l| l.as_str()).map(String::from)
                    } else { None }
                })
            });
        Some(json!({
            "ifname": ifname,
            "address": address,
            "ipv4": ipv4,
            "is_up": operstate == "UP" || operstate == "up",
        }))
    }).collect();
    Ok(normalized)
}

async fn get_local_metrics() -> Option<Value> {
    // CPU: read /proc/stat twice with a short interval
    let read_cpu = || -> Option<(u64, u64)> {
        let content = std::fs::read_to_string("/proc/stat").ok()?;
        let line = content.lines().next()?;
        let vals: Vec<u64> = line.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
        if vals.len() < 4 { return None; }
        let idle = vals[3];
        let total: u64 = vals.iter().sum();
        Some((idle, total))
    };
    let (idle1, total1) = read_cpu()?;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let (idle2, total2) = read_cpu()?;
    let diff_idle = idle2.saturating_sub(idle1) as f64;
    let diff_total = total2.saturating_sub(total1) as f64;
    let cpu_percent = if diff_total > 0.0 { (1.0 - diff_idle / diff_total) * 100.0 } else { 0.0 };

    // Memory: read /proc/meminfo
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let parse_kb = |key: &str| -> Option<u64> {
        meminfo.lines().find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
    };
    let total_kb = parse_kb("MemTotal:")?;
    let avail_kb = parse_kb("MemAvailable:")?;
    let used = (total_kb - avail_kb) * 1024;
    let total = total_kb * 1024;

    Some(json!({
        "cpuPercent": (cpu_percent * 10.0).round() / 10.0,
        "memoryUsedBytes": used,
        "memoryTotalBytes": total,
    }))
}

async fn get_local_interfaces_handler() -> Json<Value> {
    match get_local_interfaces().await {
        Ok(ifaces) => Json(json!({"success": true, "interfaces": ifaces})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

#[derive(Deserialize)]
struct UpdateLocalConfigRequest {
    #[serde(default)]
    lan_interface: Option<String>,
    #[serde(default)]
    container_storage_path: Option<String>,
}

async fn update_local_config(
    State(state): State<ApiState>,
    Json(body): Json<UpdateLocalConfigRequest>,
) -> Json<Value> {
    let cm = match &state.container_manager {
        Some(cm) => cm,
        None => return Json(json!({"success": false, "error": "Container manager not available"})),
    };
    let mut cfg = cm.get_config().await;
    if let Some(ref iface) = body.lan_interface {
        cfg.lan_interface = Some(iface.clone());
    }
    if let Some(ref sp) = body.container_storage_path {
        cfg.container_storage_path = sp.clone();
    }
    match cm.update_config(cfg).await {
        Ok(()) => Json(json!({"success": true})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

// ── Host CRUD (continued) ────────────────────────────────────────────────

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
        "lan_interface": detected_lan_interface,
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
    if id == "local" {
        return Json(json!({"success": false, "error": "Cannot delete local host"}));
    }
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
        if conn.tx.send(hr_registry::OutgoingHostMessage::Text(msg)).await.is_ok() {
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
                Ok(HostAgentMessage::Auth { token: _, host_name, version, lan_interface, container_storage_path }) => {
                    let mut data = load_hosts().await;
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

                    // Store lan_interface and container_storage_path from host agent
                    if let Some(ref id) = host_id {
                        if let Some(host) = find_host_mut(&mut data, id) {
                            let mut changed = false;
                            if let Some(ref iface) = lan_interface {
                                if host.get("lan_interface").and_then(|v| v.as_str()) != Some(iface) {
                                    host["lan_interface"] = json!(iface);
                                    changed = true;
                                }
                            }
                            if let Some(ref sp) = container_storage_path {
                                if host.get("container_storage_path").and_then(|v| v.as_str()) != Some(sp) {
                                    host["container_storage_path"] = json!(sp);
                                    changed = true;
                                }
                            }
                            if changed {
                                let _ = save_hosts(&data).await;
                                tracing::info!(host = %host_name, "Updated host config from agent: lan_interface={:?}, storage_path={:?}", lan_interface, container_storage_path);
                            }
                        }
                    }

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
    let (tx, mut rx) = mpsc::channel::<hr_registry::OutgoingHostMessage>(512);
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

    // Track which transfer_ids are being relayed (remote→remote)
    let mut relay_transfers: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Track local nspawn imports (remote→local)
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum TransferPhase { ReceivingContainer, ReceivingWorkspace }
    struct ActiveTransfer {
        container_name: String,
        storage_path: String,
        network_mode: String,
        file: tokio::fs::File,
        phase: TransferPhase,
        workspace_file: Option<tokio::fs::File>,
        total_bytes: u64,
        bytes_received: u64,
        chunk_count: u32,
        app_id: String,
        transfer_id: String,
    }
    let mut active_transfers: std::collections::HashMap<String, ActiveTransfer> = std::collections::HashMap::new();

    // Pending binary chunk metadata: (transfer_id, sequence, checksum)
    // Set when TransferChunkBinary text arrives, consumed when the next Binary frame arrives.
    let mut pending_binary_meta: Option<(String, u32, u32)> = None;

    // Heartbeat timeout: agent sends every 5s, detect offline within 10s
    let heartbeat_timeout = std::time::Duration::from_secs(10);
    let timeout_sleep = tokio::time::sleep(heartbeat_timeout);
    tokio::pin!(timeout_sleep);

    // Bidirectional message loop
    loop {
        tokio::select! {
            // Messages from registry → host-agent
            Some(msg) = rx.recv() => {
                let ws_msg = match msg {
                    hr_registry::OutgoingHostMessage::Text(m) => {
                        match serde_json::to_string(&m) {
                            Ok(t) => Message::Text(t.into()),
                            Err(_) => continue,
                        }
                    }
                    hr_registry::OutgoingHostMessage::Binary(data) => {
                        Message::Binary(data.into())
                    }
                };
                match tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    socket.send(ws_msg),
                ).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => break,    // WebSocket send error
                    Err(_) => {             // 30s timeout
                        tracing::warn!("WebSocket send timeout for host {host_id}, disconnecting");
                        break;
                    }
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
                                HostAgentMessage::ExportReady { transfer_id, container_name: _, size_bytes } => {
                                    // Check if this is a remote→remote relay
                                    if let Some((target_host_id, _cname)) = registry.get_transfer_relay_target(&transfer_id).await {
                                        tracing::info!(transfer_id = %transfer_id, target = %target_host_id, size_bytes, "Relaying ExportReady to target host");
                                        relay_transfers.insert(transfer_id.clone());
                                    } else if let Some(cname) = registry.take_transfer_container_name(&transfer_id).await {
                                        // Remote→Local nspawn import: set up file receiver
                                        let file_path = format!("/tmp/{}.tar.gz", transfer_id);
                                        match tokio::fs::File::create(&file_path).await {
                                            Ok(file) => {
                                                // Resolve storage path and network mode for local
                                                let (storage_path, network_mode) = if let Some(cm) = &state.container_manager {
                                                    let sp = cm.resolve_storage_path("local").await;
                                                    let nm = cm.resolve_network_mode("local").await
                                                        .unwrap_or_else(|_| "bridge:br-lan".to_string());
                                                    (sp, nm)
                                                } else {
                                                    ("/var/lib/machines".to_string(), "bridge:br-lan".to_string())
                                                };
                                                // Look up app_id from migration state
                                                let app_id = {
                                                    let m = state.migrations.read().await;
                                                    m.get(&transfer_id).map(|s| s.app_id.clone()).unwrap_or_default()
                                                };
                                                tracing::info!(transfer_id = %transfer_id, container = %cname, size_bytes, "Setting up local nspawn import receiver");
                                                active_transfers.insert(transfer_id.clone(), ActiveTransfer {
                                                    container_name: cname,
                                                    storage_path,
                                                    network_mode,
                                                    file,
                                                    phase: TransferPhase::ReceivingContainer,
                                                    workspace_file: None,
                                                    total_bytes: size_bytes,
                                                    bytes_received: 0,
                                                    chunk_count: 0,
                                                    app_id,
                                                    transfer_id: transfer_id.clone(),
                                                });
                                            }
                                            Err(e) => {
                                                tracing::error!(transfer_id = %transfer_id, %e, "Failed to create local import file");
                                                registry.on_host_import_failed(&host_id, &transfer_id, &format!("File creation error: {e}")).await;
                                            }
                                        }
                                    }
                                }
                                HostAgentMessage::ExportFailed { transfer_id, error } => {
                                    tracing::error!(transfer_id = %transfer_id, %error, "Host export failed");
                                    relay_transfers.remove(&transfer_id);
                                    registry.take_transfer_relay_target(&transfer_id).await;
                                    active_transfers.remove(&transfer_id);
                                    let _ = tokio::fs::remove_file(format!("/tmp/{}.tar.gz", transfer_id)).await;
                                    let _ = tokio::fs::remove_file(format!("/tmp/{}-workspace.tar.gz", transfer_id)).await;
                                    registry.on_host_export_failed(&host_id, &transfer_id, &error).await;
                                }
                                HostAgentMessage::TransferChunkBinary { transfer_id, sequence, size, checksum } => {
                                    if relay_transfers.contains(&transfer_id) {
                                        // Relay mode: forward metadata to target host
                                        if let Some((target_host_id, _)) = registry.get_transfer_relay_target(&transfer_id).await {
                                            let _ = registry.send_host_command(
                                                &target_host_id,
                                                hr_registry::protocol::HostRegistryMessage::ReceiveChunkBinary {
                                                    transfer_id: transfer_id.clone(),
                                                    sequence,
                                                    size,
                                                    checksum,
                                                },
                                            ).await;
                                        }
                                    }
                                    // Store metadata; the next Binary frame carries the actual data
                                    pending_binary_meta = Some((transfer_id, sequence, checksum));
                                }
                                HostAgentMessage::TransferComplete { transfer_id } => {
                                    if relay_transfers.remove(&transfer_id) {
                                        // Relay mode: forward TransferComplete to target host
                                        tracing::info!(transfer_id = %transfer_id, "Relaying TransferComplete to target host");
                                        if let Some((target_host_id, _)) = registry.get_transfer_relay_target(&transfer_id).await {
                                            let _ = registry.send_host_command(
                                                &target_host_id,
                                                hr_registry::protocol::HostRegistryMessage::TransferComplete {
                                                    transfer_id: transfer_id.to_string(),
                                                },
                                            ).await;
                                        }
                                        // Clean up relay target (import result will come from target host)
                                        registry.take_transfer_relay_target(&transfer_id).await;
                                    } else if let Some(mut transfer) = active_transfers.remove(&transfer_id) {
                                        // Local nspawn import: finalize
                                        use tokio::io::AsyncWriteExt;
                                        let _ = transfer.file.flush().await;
                                        drop(transfer.file);
                                        let has_workspace = transfer.phase == TransferPhase::ReceivingWorkspace;
                                        if let Some(mut ws_file) = transfer.workspace_file.take() {
                                            let _ = ws_file.flush().await;
                                            drop(ws_file);
                                        }
                                        let tid = transfer_id.clone();
                                        let reg = registry.clone();
                                        let hid = host_id.clone();
                                        tokio::spawn(async move {
                                            handle_local_nspawn_import(
                                                reg, hid, tid,
                                                transfer.container_name,
                                                transfer.storage_path,
                                                transfer.network_mode,
                                                has_workspace,
                                            ).await;
                                        });
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
                                HostAgentMessage::WorkspaceReady { transfer_id, size_bytes } => {
                                    if relay_transfers.contains(&transfer_id) {
                                        // Relay mode: forward WorkspaceReady to target host
                                        tracing::info!(transfer_id = %transfer_id, size_bytes, "Relaying WorkspaceReady to target host");
                                        if let Some((target_host_id, _)) = registry.get_transfer_relay_target(&transfer_id).await {
                                            let _ = registry.send_host_command(
                                                &target_host_id,
                                                hr_registry::protocol::HostRegistryMessage::WorkspaceReady {
                                                    transfer_id: transfer_id.to_string(),
                                                    size_bytes,
                                                },
                                            ).await;
                                        }
                                    } else if let Some(transfer) = active_transfers.get_mut(&transfer_id) {
                                        // Local import: transition to workspace phase
                                        tracing::info!(transfer_id = %transfer_id, size_bytes, "Receiving workspace for local import");
                                        let ws_path = format!("/tmp/{}-workspace.tar.gz", transfer_id);
                                        match tokio::fs::File::create(&ws_path).await {
                                            Ok(ws_file) => {
                                                transfer.phase = TransferPhase::ReceivingWorkspace;
                                                transfer.workspace_file = Some(ws_file);
                                                // Reset byte counters for workspace phase
                                                transfer.total_bytes = size_bytes;
                                                transfer.bytes_received = 0;
                                                transfer.chunk_count = 0;
                                            }
                                            Err(e) => {
                                                tracing::error!(transfer_id = %transfer_id, %e, "Failed to create workspace file for local import");
                                            }
                                        }
                                    }
                                }
                                HostAgentMessage::TerminalData { session_id, data } => {
                                    registry.send_terminal_data(&session_id, data).await;
                                }
                                HostAgentMessage::TerminalOpened { session_id } => {
                                    tracing::debug!(session_id = %session_id, "Remote terminal opened");
                                }
                                HostAgentMessage::TerminalClosed { session_id, exit_code } => {
                                    tracing::info!(session_id = %session_id, ?exit_code, "Remote terminal closed");
                                    // Send empty data to signal close to the API WS handler
                                    registry.send_terminal_data(&session_id, Vec::new()).await;
                                }
                                HostAgentMessage::Auth { .. } => {}
                                HostAgentMessage::NspawnContainerList(_) => {
                                    // TODO: track nspawn containers separately if needed
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        // Binary frame following a TransferChunkBinary metadata message
                        timeout_sleep.as_mut().reset(tokio::time::Instant::now() + heartbeat_timeout);
                        if let Some((transfer_id, _sequence, _checksum)) = pending_binary_meta.take() {
                            if relay_transfers.contains(&transfer_id) {
                                // Relay mode: forward binary data to target host
                                if let Some((target_host_id, _)) = registry.get_transfer_relay_target(&transfer_id).await {
                                    if let Err(e) = registry.send_host_binary(&target_host_id, data.to_vec()).await {
                                        tracing::error!(transfer_id = %transfer_id, %e, "Failed to relay binary chunk to target");
                                        relay_transfers.remove(&transfer_id);
                                        registry.take_transfer_relay_target(&transfer_id).await;
                                        registry.on_host_import_failed(&host_id, &transfer_id, &format!("Relay binary send failed: {e}")).await;
                                    }
                                }
                            } else if let Some(transfer) = active_transfers.get_mut(&transfer_id) {
                                // Local import mode: write binary data to file
                                use tokio::io::AsyncWriteExt;
                                let data_len = data.len() as u64;
                                let target_file = match transfer.phase {
                                    TransferPhase::ReceivingWorkspace => transfer.workspace_file.as_mut(),
                                    _ => Some(&mut transfer.file),
                                };
                                if let Some(file) = target_file {
                                    if let Err(e) = file.write_all(&data).await {
                                        tracing::error!(transfer_id = %transfer_id, %e, "Failed to write binary chunk to local file");
                                        active_transfers.remove(&transfer_id);
                                        registry.on_host_import_failed(&host_id, &transfer_id, &format!("File write error: {e}")).await;
                                    } else {
                                        // Update progress tracking
                                        transfer.bytes_received += data_len;
                                        transfer.chunk_count += 1;
                                        if transfer.chunk_count % 4 == 0 && transfer.total_bytes > 0 && !transfer.app_id.is_empty() {
                                            // Container data: 10% → 85%, workspace: 85% → 92%
                                            let (pct_start, pct_end) = match transfer.phase {
                                                TransferPhase::ReceivingContainer => (10u8, 85u8),
                                                TransferPhase::ReceivingWorkspace => (85u8, 92u8),
                                            };
                                            let ratio = (transfer.bytes_received as f64 / transfer.total_bytes as f64).min(1.0);
                                            let pct = pct_start + (ratio * (pct_end - pct_start) as f64) as u8;
                                            crate::routes::applications::update_migration_phase(
                                                &state.migrations,
                                                &state.events,
                                                &transfer.app_id,
                                                &transfer.transfer_id,
                                                hr_common::events::MigrationPhase::Importing,
                                                pct,
                                                transfer.bytes_received,
                                                transfer.total_bytes,
                                                None,
                                            ).await;
                                        }
                                    }
                                }
                            }
                        } else {
                            tracing::warn!("Received Binary frame without pending TransferChunkBinary metadata");
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

    // Clean up any pending relay transfers
    for tid in relay_transfers {
        registry.take_transfer_relay_target(&tid).await;
    }

    // Mark host offline
    update_host_status(&host_id, "offline", &state.events.host_status).await;

    registry.on_host_disconnected(&host_id).await;
    tracing::info!("Host agent disconnected: {} ({})", host_name, host_id);
}


// ── Local nspawn import for remote→local migration ─────────────────────

async fn handle_local_nspawn_import(
    registry: std::sync::Arc<hr_registry::AgentRegistry>,
    source_host_id: String,
    transfer_id: String,
    container_name: String,
    storage_path: String,
    network_mode: String,
    has_workspace: bool,
) {
    let import_path = format!("/tmp/{}.tar.gz", transfer_id);
    let ws_import_path = format!("/tmp/{}-workspace.tar.gz", transfer_id);

    // Verify the file exists and is non-empty
    match tokio::fs::metadata(&import_path).await {
        Ok(m) if m.len() == 0 => {
            tracing::error!(transfer_id = %transfer_id, "Transfer file is empty");
            registry.on_host_import_failed(&source_host_id, &transfer_id, "Transfer file is empty").await;
            let _ = tokio::fs::remove_file(&import_path).await;
            let _ = tokio::fs::remove_file(&ws_import_path).await;
            return;
        }
        Err(e) => {
            tracing::error!(transfer_id = %transfer_id, %e, "Transfer file missing");
            registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("Transfer file missing: {e}")).await;
            let _ = tokio::fs::remove_file(&ws_import_path).await;
            return;
        }
        Ok(m) => {
            tracing::info!(transfer_id = %transfer_id, size_bytes = m.len(), "Starting local nspawn import");
        }
    }

    let rootfs_dir = format!("{}/{}", storage_path, container_name);
    let ws_dir = format!("{}/{}-workspace", storage_path, container_name);

    // Create rootfs directory
    if let Err(e) = tokio::fs::create_dir_all(&rootfs_dir).await {
        tracing::error!(transfer_id = %transfer_id, %e, "Failed to create rootfs directory");
        registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("Failed to create rootfs dir: {e}")).await;
        let _ = tokio::fs::remove_file(&import_path).await;
        let _ = tokio::fs::remove_file(&ws_import_path).await;
        return;
    }

    // Extract container tar
    tracing::info!(transfer_id = %transfer_id, container = %container_name, dir = %rootfs_dir, "Extracting container tar");
    let extract = tokio::process::Command::new("tar")
        .args(["xf", &import_path, "--numeric-owner", "--xattrs", "--xattrs-include=*", "-C", &rootfs_dir])
        .output()
        .await;

    match &extract {
        Ok(output) if output.status.success() => {
            tracing::info!(transfer_id = %transfer_id, "Container tar extracted successfully");
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!(transfer_id = %transfer_id, %stderr, "Container tar extraction failed");
            registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("tar extract failed: {stderr}")).await;
            let _ = tokio::fs::remove_dir_all(&rootfs_dir).await;
            let _ = tokio::fs::remove_file(&import_path).await;
            let _ = tokio::fs::remove_file(&ws_import_path).await;
            return;
        }
        Err(e) => {
            tracing::error!(transfer_id = %transfer_id, %e, "tar command error");
            registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("tar command error: {e}")).await;
            let _ = tokio::fs::remove_dir_all(&rootfs_dir).await;
            let _ = tokio::fs::remove_file(&import_path).await;
            let _ = tokio::fs::remove_file(&ws_import_path).await;
            return;
        }
    }

    // Handle workspace
    if has_workspace {
        if let Err(e) = tokio::fs::create_dir_all(&ws_dir).await {
            tracing::warn!(transfer_id = %transfer_id, %e, "Failed to create workspace dir");
        }
        let ws_extract = tokio::process::Command::new("tar")
            .args(["xf", &ws_import_path, "--numeric-owner", "--xattrs", "--xattrs-include=*", "-C", &ws_dir])
            .output()
            .await;
        match &ws_extract {
            Ok(output) if output.status.success() => {
                tracing::info!(transfer_id = %transfer_id, "Workspace tar extracted successfully");
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!(transfer_id = %transfer_id, %stderr, "Workspace tar extraction failed, creating empty workspace");
                let _ = tokio::fs::create_dir_all(&ws_dir).await;
            }
            Err(e) => {
                tracing::warn!(transfer_id = %transfer_id, %e, "Workspace tar error, creating empty workspace");
                let _ = tokio::fs::create_dir_all(&ws_dir).await;
            }
        }
    } else {
        // Ensure workspace dir exists
        let _ = tokio::fs::create_dir_all(&ws_dir).await;
    }

    let sp = std::path::Path::new(&storage_path);

    // Write .nspawn unit
    if let Err(e) = hr_container::NspawnClient::write_nspawn_unit(&container_name, sp, &network_mode).await {
        tracing::error!(transfer_id = %transfer_id, %e, "Failed to write nspawn unit");
        registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("Failed to write nspawn unit: {e}")).await;
        let _ = tokio::fs::remove_dir_all(&rootfs_dir).await;
        let _ = tokio::fs::remove_dir_all(&ws_dir).await;
        let _ = tokio::fs::remove_file(&import_path).await;
        let _ = tokio::fs::remove_file(&ws_import_path).await;
        return;
    }

    // Write network config in rootfs
    if let Err(e) = hr_container::NspawnClient::write_network_config(&container_name, sp).await {
        tracing::error!(transfer_id = %transfer_id, %e, "Failed to write network config");
        registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("Failed to write network config: {e}")).await;
        let _ = tokio::fs::remove_dir_all(&rootfs_dir).await;
        let _ = tokio::fs::remove_dir_all(&ws_dir).await;
        let _ = tokio::fs::remove_file(&import_path).await;
        let _ = tokio::fs::remove_file(&ws_import_path).await;
        return;
    }

    // Start the container
    match hr_container::NspawnClient::start_container(&container_name).await {
        Ok(()) => {
            tracing::info!(transfer_id = %transfer_id, container = %container_name, "Nspawn container started after local import");
            registry.on_host_import_complete("local", &transfer_id, &container_name).await;
        }
        Err(e) => {
            tracing::error!(transfer_id = %transfer_id, %e, "Container start failed after import");
            registry.on_host_import_failed(&source_host_id, &transfer_id, &format!("Start failed: {e}")).await;
        }
    }

    // Cleanup transfer files
    let _ = tokio::fs::remove_file(&import_path).await;
    let _ = tokio::fs::remove_file(&ws_import_path).await;
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
{lan_line}container_storage_path = "/var/lib/machines"
container_runtime = "nspawn"
"#,
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
apt-get install -y systemd-container debootstrap && \
systemctl daemon-reload && \
systemctl enable --now hr-host-agent"#,
    );

    // Escape single quotes in inner_cmds for shell wrapping
    let escaped = inner_cmds.replace('\'', "'\\''");

    let setup_cmd = format!("echo '{password}' | sudo -S bash -c '{escaped}'");

    ssh_command(host, port, user, &setup_cmd).await?;

    Ok(())
}
