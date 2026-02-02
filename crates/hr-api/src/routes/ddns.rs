use axum::{
    extract::State,
    routing::{get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/status", get(status))
        .route("/update", post(force_update))
        .route("/token", put(update_token))
        .route("/config", put(update_config))
}

async fn status(State(state): State<ApiState>) -> Json<Value> {
    let env = &state.env;
    let interface = &env.cf_interface;

    // Get current IPv6 address
    let ipv6 = get_ipv6_address(interface).await;

    // Get Cloudflare record if configured
    let cf_ip = if env.cf_api_token.is_some() && env.cf_zone_id.is_some() && env.cf_record_name.is_some()
    {
        get_cloudflare_ip(env).await.ok()
    } else {
        None
    };

    let configured = env.cf_api_token.is_some()
        && env.cf_zone_id.is_some()
        && env.cf_record_name.is_some();

    // Read last update log
    let log = tokio::fs::read_to_string("/data/ddns.log")
        .await
        .unwrap_or_default();
    let log_lines: Vec<&str> = log.lines().rev().take(20).collect();

    // Mask the API token for display (show last 4 chars only)
    let masked_token = env.cf_api_token.as_ref().map(|t| {
        if t.len() > 4 {
            format!("****{}", &t[t.len()-4..])
        } else {
            "****".to_string()
        }
    });

    // Parse last update info from logs
    let last_update = log.lines().rev().find(|l| l.contains("Updated ")).map(|l| {
        // Extract timestamp from "[2024-01-01T00:00:00Z] Updated ..."
        l.trim_start_matches('[').split(']').next().unwrap_or("").to_string()
    });

    Json(json!({
        "success": true,
        "status": {
            "configured": configured,
            "interface": interface,
            "currentIpv6": ipv6,
            "cloudflareIp": cf_ip,
            "inSync": ipv6.as_deref() == cf_ip.as_deref(),
            "lastUpdate": last_update,
            "lastIp": cf_ip,
            "config": {
                "recordName": env.cf_record_name,
                "zoneId": env.cf_zone_id,
                "apiToken": masked_token,
                "proxied": env.cf_proxied,
            },
            "logs": log_lines
        }
    }))
}

async fn force_update(State(state): State<ApiState>) -> Json<Value> {
    let env = &state.env;

    let token = match &env.cf_api_token {
        Some(t) => t,
        None => return Json(json!({"success": false, "error": "Token Cloudflare non configure"})),
    };
    let zone_id = match &env.cf_zone_id {
        Some(z) => z,
        None => return Json(json!({"success": false, "error": "Zone ID non configure"})),
    };
    let record_name = match &env.cf_record_name {
        Some(r) => r,
        None => return Json(json!({"success": false, "error": "Nom d'enregistrement non configure"})),
    };

    let ipv6 = match get_ipv6_address(&env.cf_interface).await {
        Some(ip) => ip,
        None => return Json(json!({"success": false, "error": "Impossible de determiner l'adresse IPv6"})),
    };

    match update_cloudflare_record(token, zone_id, record_name, &ipv6, env.cf_proxied).await {
        Ok(()) => {
            log_ddns(&format!("Updated {} to {}", record_name, ipv6)).await;
            Json(json!({"success": true, "ipv6": ipv6}))
        }
        Err(e) => {
            log_ddns(&format!("Update failed: {}", e)).await;
            Json(json!({"success": false, "error": e}))
        }
    }
}

#[derive(Deserialize)]
struct UpdateTokenRequest {
    token: String,
}

async fn update_token(Json(body): Json<UpdateTokenRequest>) -> Json<Value> {
    // We can't update the env config at runtime in the same way Node.js does.
    // Instead, we write to the .env file for next restart.
    let env_path = "/opt/homeroute/.env";
    let content = tokio::fs::read_to_string(env_path)
        .await
        .unwrap_or_default();

    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    let mut found = false;
    for line in &mut lines {
        if line.starts_with("CF_API_TOKEN=") {
            *line = format!("CF_API_TOKEN={}", body.token);
            found = true;
        }
    }
    if !found {
        lines.push(format!("CF_API_TOKEN={}", body.token));
    }

    if let Err(e) = tokio::fs::write(env_path, lines.join("\n") + "\n").await {
        return Json(json!({"success": false, "error": e.to_string()}));
    }

    Json(json!({"success": true, "message": "Token mis a jour. Redemarrez le service pour appliquer."}))
}

#[derive(Deserialize)]
struct UpdateConfigRequest {
    zone_id: Option<String>,
    proxied: Option<bool>,
}

async fn update_config(Json(body): Json<UpdateConfigRequest>) -> Json<Value> {
    let env_path = "/opt/homeroute/.env";
    let content = tokio::fs::read_to_string(env_path)
        .await
        .unwrap_or_default();

    let mut lines: Vec<String> = content.lines().map(String::from).collect();

    if let Some(zone_id) = &body.zone_id {
        let mut found = false;
        for line in &mut lines {
            if line.starts_with("CF_ZONE_ID=") {
                *line = format!("CF_ZONE_ID={}", zone_id);
                found = true;
            }
        }
        if !found {
            lines.push(format!("CF_ZONE_ID={}", zone_id));
        }
    }

    if let Some(proxied) = body.proxied {
        let mut found = false;
        for line in &mut lines {
            if line.starts_with("CF_PROXIED=") {
                *line = format!("CF_PROXIED={}", proxied);
                found = true;
            }
        }
        if !found {
            lines.push(format!("CF_PROXIED={}", proxied));
        }
    }

    if let Err(e) = tokio::fs::write(env_path, lines.join("\n") + "\n").await {
        return Json(json!({"success": false, "error": e.to_string()}));
    }

    Json(json!({"success": true, "message": "Configuration mise a jour. Redemarrez le service pour appliquer."}))
}

async fn get_ipv6_address(interface: &str) -> Option<String> {
    let output = tokio::process::Command::new("ip")
        .args(["-6", "addr", "show", interface, "scope", "global"])
        .output()
        .await
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with("inet6") && !line.contains("temporary") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(addr) = parts.get(1) {
                if let Some(ip) = addr.split('/').next() {
                    return Some(ip.to_string());
                }
            }
        }
    }
    None
}

async fn get_cloudflare_ip(
    env: &hr_common::config::EnvConfig,
) -> Result<String, String> {
    let token = env.cf_api_token.as_ref().ok_or("No token")?;
    let zone_id = env.cf_zone_id.as_ref().ok_or("No zone ID")?;
    let record_name = env.cf_record_name.as_ref().ok_or("No record name")?;

    let client = reqwest::Client::new();
    let url = format!(
        "https://api.cloudflare.com/client/v4/zones/{}/dns_records?type=AAAA&name={}",
        zone_id, record_name
    );

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let body: Value = resp.json().await.map_err(|e| e.to_string())?;

    // Check for Cloudflare API errors
    if let Some(false) = body.get("success").and_then(|s| s.as_bool()) {
        let errors = body.get("errors").and_then(|e| e.as_array())
            .map(|arr| arr.iter()
                .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                .collect::<Vec<_>>()
                .join(", "))
            .unwrap_or_else(|| "Unknown error".to_string());
        return Err(format!("Cloudflare API: {}", errors));
    }

    body.get("result")
        .and_then(|r| r.as_array())
        .and_then(|arr| arr.first())
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_str())
        .map(String::from)
        .ok_or_else(|| "Record not found".to_string())
}

async fn update_cloudflare_record(
    token: &str,
    zone_id: &str,
    record_name: &str,
    ipv6: &str,
    proxied: bool,
) -> Result<(), String> {
    let client = reqwest::Client::new();

    // First get the record ID
    let list_url = format!(
        "https://api.cloudflare.com/client/v4/zones/{}/dns_records?type=AAAA&name={}",
        zone_id, record_name
    );

    let resp = client
        .get(&list_url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let body: Value = resp.json().await.map_err(|e| e.to_string())?;

    // Check for Cloudflare API errors
    if let Some(false) = body.get("success").and_then(|s| s.as_bool()) {
        let errors = body.get("errors").and_then(|e| e.as_array())
            .map(|arr| arr.iter()
                .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                .collect::<Vec<_>>()
                .join(", "))
            .unwrap_or_else(|| "Unknown error".to_string());
        return Err(format!("Cloudflare API: {}", errors));
    }

    let records = body
        .get("result")
        .and_then(|r| r.as_array())
        .ok_or("Invalid response from Cloudflare")?;

    if let Some(record) = records.first() {
        // Update existing record
        let record_id = record
            .get("id")
            .and_then(|i| i.as_str())
            .ok_or("No record ID")?;

        let update_url = format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records/{}",
            zone_id, record_id
        );

        let resp = client
            .put(&update_url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({
                "type": "AAAA",
                "name": record_name,
                "content": ipv6,
                "ttl": 120,
                "proxied": proxied
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Cloudflare API error: {}", body));
        }
    } else {
        // Create new record
        let create_url = format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records",
            zone_id
        );

        let resp = client
            .post(&create_url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({
                "type": "AAAA",
                "name": record_name,
                "content": ipv6,
                "ttl": 120,
                "proxied": proxied
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Cloudflare API error: {}", body));
        }
    }

    Ok(())
}

async fn log_ddns(message: &str) {
    let timestamp = chrono::Utc::now().to_rfc3339();
    let entry = format!("[{}] {}\n", timestamp, message);
    if let Ok(mut f) = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/data/ddns.log")
        .await
    {
        use tokio::io::AsyncWriteExt;
        let _ = f.write_all(entry.as_bytes()).await;
    }
}
