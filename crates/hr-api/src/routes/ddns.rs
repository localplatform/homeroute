use axum::{
    extract::State,
    routing::{get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use hr_registry::cloudflare;

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
    let cf_ip = match (&env.cf_api_token, &env.cf_zone_id, &env.cf_record_name) {
        (Some(token), Some(zone_id), Some(record_name)) => {
            cloudflare::get_aaaa_record_content(token, zone_id, record_name)
                .await
                .ok()
        }
        _ => None,
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

    // Cloud relay mode: update A record with VPS IPv4
    if env.cloud_relay_enabled {
        let vps_ipv4 = match &env.cloud_relay_host {
            Some(host) => host.clone(),
            None => return Json(json!({"success": false, "error": "Cloud relay enabled but CLOUD_RELAY_HOST not configured"})),
        };

        match cloudflare::upsert_a_record(token, zone_id, record_name, &vps_ipv4, env.cf_proxied).await {
            Ok(_record_id) => {
                log_ddns(&format!("Updated {} to A {} (relay mode)", record_name, vps_ipv4)).await;
                Json(json!({"success": true, "ipv4": vps_ipv4, "mode": "relay"}))
            }
            Err(e) => {
                log_ddns(&format!("Update failed (relay): {}", e)).await;
                Json(json!({"success": false, "error": e}))
            }
        }
    } else {
        // Direct mode: update AAAA record with on-prem IPv6
        let ipv6 = match get_ipv6_address(&env.cf_interface).await {
            Some(ip) => ip,
            None => return Json(json!({"success": false, "error": "Impossible de determiner l'adresse IPv6"})),
        };

        match cloudflare::upsert_aaaa_record(token, zone_id, record_name, &ipv6, env.cf_proxied).await {
            Ok(_record_id) => {
                log_ddns(&format!("Updated {} to AAAA {}", record_name, ipv6)).await;
                Json(json!({"success": true, "ipv6": ipv6, "mode": "direct"}))
            }
            Err(e) => {
                log_ddns(&format!("Update failed: {}", e)).await;
                Json(json!({"success": false, "error": e}))
            }
        }
    }
}

#[derive(Deserialize)]
struct UpdateTokenRequest {
    token: String,
}

async fn update_token(Json(body): Json<UpdateTokenRequest>) -> Json<Value> {
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
