use axum::{
    extract::Path,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::ApiState;

const SERVERS_FILE: &str = "/data/servers.json";
const SSH_KEY_PATH: &str = "/data/ssh/id_rsa";
const SSH_PUB_KEY_PATH: &str = "/data/ssh/id_rsa.pub";

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/", get(list_servers).post(add_server))
        .route("/groups", get(list_groups))
        .route("/{id}", get(get_server).put(update_server).delete(delete_server))
        .route("/{id}/test", post(test_connection))
        .route("/{id}/interfaces", get(get_interfaces))
        .route("/{id}/refresh-interfaces", post(refresh_interfaces))
        .route("/{id}/info", get(get_server_info))
}

async fn load_servers() -> Value {
    match tokio::fs::read_to_string(SERVERS_FILE).await {
        Ok(content) => serde_json::from_str(&content).unwrap_or(json!({"servers": []})),
        Err(_) => json!({"servers": []}),
    }
}

async fn save_servers(data: &Value) -> Result<(), String> {
    let content = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    let tmp = format!("{}.tmp", SERVERS_FILE);
    tokio::fs::write(&tmp, &content)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(&tmp, SERVERS_FILE)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn list_servers() -> Json<Value> {
    let data = load_servers().await;
    let servers = data.get("servers").cloned().unwrap_or(json!([]));
    Json(json!({"success": true, "servers": servers}))
}

async fn list_groups() -> Json<Value> {
    let data = load_servers().await;
    let mut groups = std::collections::BTreeSet::new();
    if let Some(servers) = data.get("servers").and_then(|s| s.as_array()) {
        for server in servers {
            if let Some(sg) = server.get("groups").and_then(|g| g.as_array()) {
                for g in sg {
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

async fn get_server(Path(id): Path<String>) -> Json<Value> {
    let data = load_servers().await;
    if let Some(servers) = data.get("servers").and_then(|s| s.as_array()) {
        if let Some(server) = servers.iter().find(|s| s.get("id").and_then(|i| i.as_str()) == Some(&id)) {
            return Json(json!({"success": true, "server": server}));
        }
    }
    Json(json!({"success": false, "error": "Serveur non trouve"}))
}

#[derive(Deserialize)]
struct AddServerRequest {
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

async fn add_server(Json(body): Json<AddServerRequest>) -> Json<Value> {
    // Ensure SSH key pair exists
    if let Err(e) = ensure_ssh_key().await {
        return Json(json!({"success": false, "error": format!("SSH key error: {}", e)}));
    }

    // If password provided, setup key-based auth
    if let Some(ref password) = body.password {
        if let Err(e) = setup_ssh_key(&body.host, body.port, &body.username, password).await {
            return Json(json!({"success": false, "error": format!("SSH setup failed: {}", e)}));
        }
    }

    // Get remote interfaces
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

    let id = uuid::Uuid::new_v4().to_string();
    let server = json!({
        "id": id,
        "name": body.name,
        "host": body.host,
        "port": body.port,
        "username": body.username,
        "interface": body.interface,
        "mac": mac,
        "groups": body.groups,
        "interfaces": interfaces.unwrap_or_default(),
        "createdAt": chrono::Utc::now().to_rfc3339()
    });

    let mut data = load_servers().await;
    let servers = data.get_mut("servers").and_then(|s| s.as_array_mut());
    match servers {
        Some(arr) => arr.push(server.clone()),
        None => data["servers"] = json!([server]),
    }

    if let Err(e) = save_servers(&data).await {
        return Json(json!({"success": false, "error": e}));
    }

    Json(json!({"success": true, "server": server}))
}

async fn update_server(Path(id): Path<String>, Json(updates): Json<Value>) -> Json<Value> {
    let mut data = load_servers().await;
    if let Some(servers) = data.get_mut("servers").and_then(|s| s.as_array_mut()) {
        if let Some(server) = servers.iter_mut().find(|s| s.get("id").and_then(|i| i.as_str()) == Some(&id)) {
            if let Some(obj) = updates.as_object() {
                for (k, v) in obj {
                    if k != "id" {
                        server[k] = v.clone();
                    }
                }
            }
            server["updatedAt"] = json!(chrono::Utc::now().to_rfc3339());
        } else {
            return Json(json!({"success": false, "error": "Serveur non trouve"}));
        }
    }

    if let Err(e) = save_servers(&data).await {
        return Json(json!({"success": false, "error": e}));
    }
    Json(json!({"success": true}))
}

async fn delete_server(Path(id): Path<String>) -> Json<Value> {
    let mut data = load_servers().await;
    if let Some(servers) = data.get_mut("servers").and_then(|s| s.as_array_mut()) {
        servers.retain(|s| s.get("id").and_then(|i| i.as_str()) != Some(&id));
    }
    if let Err(e) = save_servers(&data).await {
        return Json(json!({"success": false, "error": e}));
    }
    Json(json!({"success": true}))
}

async fn test_connection(Path(id): Path<String>) -> Json<Value> {
    let data = load_servers().await;
    let server = match data.get("servers").and_then(|s| s.as_array()).and_then(|arr| {
        arr.iter().find(|s| s.get("id").and_then(|i| i.as_str()) == Some(&id))
    }) {
        Some(s) => s,
        None => return Json(json!({"success": false, "error": "Serveur non trouve"})),
    };

    let host = server.get("host").and_then(|h| h.as_str()).unwrap_or("");
    let port = server.get("port").and_then(|p| p.as_u64()).unwrap_or(22) as u16;
    let user = server.get("username").and_then(|u| u.as_str()).unwrap_or("root");

    match ssh_command(host, port, user, "echo ok").await {
        Ok(output) => Json(json!({"success": true, "output": output.trim()})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn get_interfaces(Path(id): Path<String>) -> Json<Value> {
    let data = load_servers().await;
    let server = match data.get("servers").and_then(|s| s.as_array()).and_then(|arr| {
        arr.iter().find(|s| s.get("id").and_then(|i| i.as_str()) == Some(&id))
    }) {
        Some(s) => s,
        None => return Json(json!({"success": false, "error": "Serveur non trouve"})),
    };

    let cached = server.get("interfaces").cloned().unwrap_or(json!([]));
    Json(json!({"success": true, "interfaces": cached}))
}

async fn refresh_interfaces(Path(id): Path<String>) -> Json<Value> {
    let mut data = load_servers().await;
    let server = match data.get_mut("servers").and_then(|s| s.as_array_mut()).and_then(|arr| {
        arr.iter_mut().find(|s| s.get("id").and_then(|i| i.as_str()) == Some(id.as_str()))
    }) {
        Some(s) => s,
        None => return Json(json!({"success": false, "error": "Serveur non trouve"})),
    };

    let host = server.get("host").and_then(|h| h.as_str()).unwrap_or("").to_string();
    let port = server.get("port").and_then(|p| p.as_u64()).unwrap_or(22) as u16;
    let user = server.get("username").and_then(|u| u.as_str()).unwrap_or("root").to_string();

    match get_remote_interfaces(&host, port, &user).await {
        Ok(ifaces) => {
            server["interfaces"] = json!(ifaces);
            let _ = save_servers(&data).await;
            Json(json!({"success": true, "interfaces": ifaces}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn get_server_info(Path(id): Path<String>) -> Json<Value> {
    let data = load_servers().await;
    let server = match data.get("servers").and_then(|s| s.as_array()).and_then(|arr| {
        arr.iter().find(|s| s.get("id").and_then(|i| i.as_str()) == Some(&id))
    }) {
        Some(s) => s,
        None => return Json(json!({"success": false, "error": "Serveur non trouve"})),
    };

    let host = server.get("host").and_then(|h| h.as_str()).unwrap_or("");
    let port = server.get("port").and_then(|p| p.as_u64()).unwrap_or(22) as u16;
    let user = server.get("username").and_then(|u| u.as_str()).unwrap_or("root");

    let info_cmd = "hostname && uname -r && uptime -p && free -b | head -2 && df -B1 / | tail -1";
    match ssh_command(host, port, user, info_cmd).await {
        Ok(output) => Json(json!({"success": true, "info": output})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

// --- SSH helpers ---

async fn ensure_ssh_key() -> Result<(), String> {
    if tokio::fs::metadata(SSH_KEY_PATH).await.is_ok() {
        return Ok(());
    }

    // Create directory
    let _ = tokio::fs::create_dir_all("/data/ssh").await;

    let output = tokio::process::Command::new("ssh-keygen")
        .args(["-t", "rsa", "-b", "4096", "-f", SSH_KEY_PATH, "-N", ""])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    // Set permissions
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

    // Use sshpass to copy the key
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
            &format!("root@{}", host), // Always connect as root
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

    // Try JSON parse first
    if let Ok(ifaces) = serde_json::from_str::<Vec<Value>>(&output) {
        return Ok(ifaces);
    }

    // Fallback: basic text parsing
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
