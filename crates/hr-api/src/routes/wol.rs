use axum::{
    extract::Path,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::ApiState;

const SCHEDULES_FILE: &str = "/data/wol-schedules.json";

pub fn router() -> Router<ApiState> {
    Router::new()
        // Power actions
        .route("/{id}/wake", post(wake))
        .route("/{id}/shutdown", post(shutdown))
        .route("/{id}/reboot", post(reboot))
        .route("/bulk/wake", post(bulk_wake))
        .route("/bulk/shutdown", post(bulk_shutdown))
        // Schedules
        .route("/schedules", get(list_schedules).post(create_schedule))
        .route(
            "/schedules/{id}",
            get(get_schedule).put(update_schedule).delete(delete_schedule),
        )
        .route("/schedules/{id}/toggle", post(toggle_schedule))
        .route("/schedules/{id}/execute", post(execute_schedule))
}

// --- Power actions ---

async fn wake(Path(id): Path<String>) -> Json<Value> {
    let server = match get_server_by_id(&id).await {
        Some(s) => s,
        None => return Json(json!({"success": false, "error": "Serveur non trouve"})),
    };

    let mac = match server.get("mac").and_then(|m| m.as_str()) {
        Some(m) => m,
        None => return Json(json!({"success": false, "error": "Adresse MAC non configuree"})),
    };

    match send_wol(mac).await {
        Ok(()) => Json(json!({"success": true, "action": "wake", "mac": mac})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn shutdown(Path(id): Path<String>) -> Json<Value> {
    let server = match get_server_by_id(&id).await {
        Some(s) => s,
        None => return Json(json!({"success": false, "error": "Serveur non trouve"})),
    };

    ssh_power_action(&server, "poweroff || shutdown -h now").await
}

async fn reboot(Path(id): Path<String>) -> Json<Value> {
    let server = match get_server_by_id(&id).await {
        Some(s) => s,
        None => return Json(json!({"success": false, "error": "Serveur non trouve"})),
    };

    ssh_power_action(&server, "reboot").await
}

#[derive(Deserialize)]
struct BulkRequest {
    #[serde(rename = "serverIds")]
    server_ids: Vec<String>,
}

async fn bulk_wake(Json(body): Json<BulkRequest>) -> Json<Value> {
    let mut results = Vec::new();
    for id in &body.server_ids {
        let server = get_server_by_id(id).await;
        if let Some(server) = server {
            if let Some(mac) = server.get("mac").and_then(|m| m.as_str()) {
                let success = send_wol(mac).await.is_ok();
                results.push(json!({"id": id, "success": success}));
            } else {
                results.push(json!({"id": id, "success": false, "error": "No MAC"}));
            }
        } else {
            results.push(json!({"id": id, "success": false, "error": "Not found"}));
        }
    }
    Json(json!({"success": true, "results": results}))
}

async fn bulk_shutdown(Json(body): Json<BulkRequest>) -> Json<Value> {
    let mut results = Vec::new();
    for id in &body.server_ids {
        let server = get_server_by_id(id).await;
        if let Some(server) = server {
            let result = ssh_power_action(&server, "poweroff || shutdown -h now").await;
            results.push(json!({"id": id, "result": result.0}));
        } else {
            results.push(json!({"id": id, "success": false, "error": "Not found"}));
        }
    }
    Json(json!({"success": true, "results": results}))
}

// --- Schedules ---

async fn load_schedules() -> Value {
    match tokio::fs::read_to_string(SCHEDULES_FILE).await {
        Ok(content) => serde_json::from_str(&content).unwrap_or(json!({"schedules": []})),
        Err(_) => json!({"schedules": []}),
    }
}

async fn save_schedules(data: &Value) -> Result<(), String> {
    let content = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    let tmp = format!("{}.tmp", SCHEDULES_FILE);
    tokio::fs::write(&tmp, &content)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(&tmp, SCHEDULES_FILE)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn list_schedules() -> Json<Value> {
    let data = load_schedules().await;
    let schedules = data.get("schedules").cloned().unwrap_or(json!([]));
    Json(json!({"success": true, "schedules": schedules}))
}

async fn get_schedule(Path(id): Path<String>) -> Json<Value> {
    let data = load_schedules().await;
    if let Some(schedules) = data.get("schedules").and_then(|s| s.as_array()) {
        if let Some(schedule) = schedules.iter().find(|s| s.get("id").and_then(|i| i.as_str()) == Some(&id)) {
            return Json(json!({"success": true, "schedule": schedule}));
        }
    }
    Json(json!({"success": false, "error": "Schedule non trouve"}))
}

#[derive(Deserialize)]
struct CreateScheduleRequest {
    #[serde(rename = "serverId")]
    server_id: String,
    action: String,
    cron: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool {
    true
}

async fn create_schedule(Json(body): Json<CreateScheduleRequest>) -> Json<Value> {
    // Validate action
    if !["wake", "shutdown", "reboot"].contains(&body.action.as_str()) {
        return Json(json!({"success": false, "error": "Action invalide. Doit etre: wake, shutdown, ou reboot"}));
    }

    // Validate server exists
    let server = match get_server_by_id(&body.server_id).await {
        Some(s) => s,
        None => return Json(json!({"success": false, "error": "Serveur non trouve"})),
    };

    let server_name = server
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("unknown");

    let id = uuid::Uuid::new_v4().to_string();
    let schedule = json!({
        "id": id,
        "serverId": body.server_id,
        "serverName": server_name,
        "action": body.action,
        "cron": body.cron,
        "description": body.description.unwrap_or_else(|| format!("{} {}", body.action, server_name)),
        "enabled": body.enabled,
        "createdAt": chrono::Utc::now().to_rfc3339(),
        "updatedAt": chrono::Utc::now().to_rfc3339(),
        "lastRun": null,
        "nextRun": null
    });

    let mut data = load_schedules().await;
    let schedules = data.get_mut("schedules").and_then(|s| s.as_array_mut());
    match schedules {
        Some(arr) => arr.push(schedule.clone()),
        None => data["schedules"] = json!([schedule]),
    }

    if let Err(e) = save_schedules(&data).await {
        return Json(json!({"success": false, "error": e}));
    }

    Json(json!({"success": true, "schedule": schedule}))
}

async fn update_schedule(Path(id): Path<String>, Json(updates): Json<Value>) -> Json<Value> {
    let mut data = load_schedules().await;
    if let Some(schedules) = data.get_mut("schedules").and_then(|s| s.as_array_mut()) {
        if let Some(schedule) = schedules.iter_mut().find(|s| s.get("id").and_then(|i| i.as_str()) == Some(&id)) {
            if let Some(obj) = updates.as_object() {
                for (k, v) in obj {
                    if k != "id" {
                        schedule[k] = v.clone();
                    }
                }
            }
            schedule["updatedAt"] = json!(chrono::Utc::now().to_rfc3339());
        } else {
            return Json(json!({"success": false, "error": "Schedule non trouve"}));
        }
    }

    if let Err(e) = save_schedules(&data).await {
        return Json(json!({"success": false, "error": e}));
    }
    Json(json!({"success": true}))
}

async fn delete_schedule(Path(id): Path<String>) -> Json<Value> {
    let mut data = load_schedules().await;
    if let Some(schedules) = data.get_mut("schedules").and_then(|s| s.as_array_mut()) {
        schedules.retain(|s| s.get("id").and_then(|i| i.as_str()) != Some(&id));
    }
    if let Err(e) = save_schedules(&data).await {
        return Json(json!({"success": false, "error": e}));
    }
    Json(json!({"success": true}))
}

async fn toggle_schedule(Path(id): Path<String>) -> Json<Value> {
    let mut data = load_schedules().await;
    if let Some(schedules) = data.get_mut("schedules").and_then(|s| s.as_array_mut()) {
        if let Some(schedule) = schedules.iter_mut().find(|s| s.get("id").and_then(|i| i.as_str()) == Some(&id)) {
            let current = schedule
                .get("enabled")
                .and_then(|e| e.as_bool())
                .unwrap_or(true);
            schedule["enabled"] = json!(!current);
        }
    }
    if let Err(e) = save_schedules(&data).await {
        return Json(json!({"success": false, "error": e}));
    }
    Json(json!({"success": true}))
}

async fn execute_schedule(Path(id): Path<String>) -> Json<Value> {
    let data = load_schedules().await;
    let schedule = match data
        .get("schedules")
        .and_then(|s| s.as_array())
        .and_then(|arr| arr.iter().find(|s| s.get("id").and_then(|i| i.as_str()) == Some(&id)))
    {
        Some(s) => s.clone(),
        None => return Json(json!({"success": false, "error": "Schedule non trouve"})),
    };

    let server_id = schedule
        .get("serverId")
        .and_then(|s| s.as_str())
        .unwrap_or("");
    let action = schedule
        .get("action")
        .and_then(|a| a.as_str())
        .unwrap_or("");

    let result = match action {
        "wake" => wake(Path(server_id.to_string())).await,
        "shutdown" => shutdown(Path(server_id.to_string())).await,
        "reboot" => reboot(Path(server_id.to_string())).await,
        _ => return Json(json!({"success": false, "error": "Action inconnue"})),
    };

    // Update lastRun
    let mut data = load_schedules().await;
    if let Some(schedules) = data.get_mut("schedules").and_then(|s| s.as_array_mut()) {
        if let Some(s) = schedules.iter_mut().find(|s| s.get("id").and_then(|i| i.as_str()) == Some(&id)) {
            s["lastRun"] = json!(chrono::Utc::now().to_rfc3339());
        }
    }
    let _ = save_schedules(&data).await;

    result
}

// --- Helpers ---

async fn get_server_by_id(id: &str) -> Option<Value> {
    let content = tokio::fs::read_to_string("/data/servers.json").await.ok()?;
    let data: Value = serde_json::from_str(&content).ok()?;
    data.get("servers")?
        .as_array()?
        .iter()
        .find(|s| s.get("id").and_then(|i| i.as_str()) == Some(id))
        .cloned()
}

async fn send_wol(mac: &str) -> Result<(), String> {
    // Parse MAC address
    let mac_bytes: Vec<u8> = mac
        .split(':')
        .filter_map(|b| u8::from_str_radix(b, 16).ok())
        .collect();

    if mac_bytes.len() != 6 {
        return Err("Adresse MAC invalide".to_string());
    }

    // Build magic packet: 6x 0xFF + 16x MAC
    let mut packet = vec![0xFFu8; 6];
    for _ in 0..16 {
        packet.extend_from_slice(&mac_bytes);
    }

    // Send via UDP broadcast
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| e.to_string())?;
    socket
        .set_broadcast(true)
        .map_err(|e| e.to_string())?;
    socket
        .send_to(&packet, "255.255.255.255:9")
        .await
        .map_err(|e| e.to_string())?;

    // Also try sending to the subnet broadcast (10.0.0.255)
    let _ = socket.send_to(&packet, "10.0.0.255:9").await;

    Ok(())
}

async fn ssh_power_action(server: &Value, command: &str) -> Json<Value> {
    let host = server.get("host").and_then(|h| h.as_str()).unwrap_or("");
    let port = server.get("port").and_then(|p| p.as_u64()).unwrap_or(22);
    let user = server.get("username").and_then(|u| u.as_str()).unwrap_or("root");

    let output = tokio::process::Command::new("ssh")
        .args([
            "-i", "/data/ssh/id_rsa",
            "-o", "StrictHostKeyChecking=no",
            "-o", "ConnectTimeout=15",
            "-o", "BatchMode=yes",
            "-p", &port.to_string(),
            &format!("root@{}", host),
            command,
        ])
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() || o.status.code() == Some(255) => {
            // 255 is expected for power-off (connection dropped)
            Json(json!({"success": true, "action": command.split_whitespace().next().unwrap_or(command)}))
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            Json(json!({"success": false, "error": format!("SSH error: {}", stderr)}))
        }
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}
