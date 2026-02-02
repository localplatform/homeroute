use axum::{
    extract::{Query, State},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/stats", get(stats))
        .route("/whitelist", get(get_whitelist).post(add_whitelist))
        .route("/whitelist/{domain}", delete(remove_whitelist))
        .route("/update", post(trigger_update))
        .route("/search", get(search))
}

async fn stats(State(state): State<ApiState>) -> Json<Value> {
    let engine = state.adblock.read().await;
    let dns = state.dns.read().await;

    // Read sources from config for frontend display
    let sources = read_adblock_sources(&state).await;

    // Check cache file mtime for lastUpdate
    let last_update = tokio::fs::metadata("/var/lib/server-dashboard/adblock/domains.json")
        .await
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64
        });

    Json(json!({
        "success": true,
        "stats": {
            "domainCount": engine.domain_count(),
            "sources": sources,
            "lastUpdate": last_update,
            "enabled": dns.adblock_enabled
        }
    }))
}

async fn get_whitelist(State(state): State<ApiState>) -> Json<Value> {
    let engine = state.adblock.read().await;
    let domains = engine.whitelist_domains();
    Json(json!({"success": true, "domains": domains}))
}

#[derive(Deserialize)]
struct AddWhitelistRequest {
    domain: String,
}

async fn add_whitelist(
    State(state): State<ApiState>,
    Json(body): Json<AddWhitelistRequest>,
) -> Json<Value> {
    let domain = body.domain.to_lowercase().trim().to_string();
    if domain.is_empty() {
        return Json(json!({"success": false, "error": "Domain requis"}));
    }

    // Read current whitelist, add domain, save to config file
    let config_path = &state.dns_dhcp_config_path;
    let content = match tokio::fs::read_to_string(config_path).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": format!("Config read error: {}", e)})),
    };

    let mut config: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => return Json(json!({"success": false, "error": format!("Config parse error: {}", e)})),
    };

    // Update whitelist in config
    let adblock = config.get_mut("adblock").and_then(|a| a.as_object_mut());
    if let Some(adblock) = adblock {
        let whitelist = adblock
            .entry("whitelist")
            .or_insert_with(|| json!([]))
            .as_array_mut();
        if let Some(wl) = whitelist {
            let domain_val = json!(domain);
            if !wl.contains(&domain_val) {
                wl.push(domain_val);
            }
        }
    }

    // Save config
    if let Ok(new_content) = serde_json::to_string_pretty(&config) {
        let tmp = config_path.with_extension("json.tmp");
        let _ = tokio::fs::write(&tmp, &new_content).await;
        let _ = tokio::fs::rename(&tmp, config_path).await;
    }

    // Update engine in memory
    {
        let mut engine = state.adblock.write().await;
        let mut domains = engine.whitelist_domains();
        if !domains.contains(&domain) {
            domains.push(domain.clone());
        }
        engine.set_whitelist(domains);
    }

    Json(json!({"success": true, "domain": domain}))
}

async fn remove_whitelist(
    State(state): State<ApiState>,
    axum::extract::Path(domain): axum::extract::Path<String>,
) -> Json<Value> {
    let domain = domain.to_lowercase();

    // Update config file
    let config_path = &state.dns_dhcp_config_path;
    if let Ok(content) = tokio::fs::read_to_string(config_path).await {
        if let Ok(mut config) = serde_json::from_str::<Value>(&content) {
            if let Some(adblock) = config.get_mut("adblock").and_then(|a| a.as_object_mut()) {
                if let Some(wl) = adblock.get_mut("whitelist").and_then(|w| w.as_array_mut()) {
                    wl.retain(|d| d.as_str() != Some(&domain));
                }
            }
            if let Ok(new_content) = serde_json::to_string_pretty(&config) {
                let tmp = config_path.with_extension("json.tmp");
                let _ = tokio::fs::write(&tmp, &new_content).await;
                let _ = tokio::fs::rename(&tmp, config_path).await;
            }
        }
    }

    // Update engine in memory
    {
        let mut engine = state.adblock.write().await;
        let mut domains = engine.whitelist_domains();
        domains.retain(|d| d != &domain);
        engine.set_whitelist(domains);
    }

    Json(json!({"success": true}))
}

async fn trigger_update(State(state): State<ApiState>) -> Json<Value> {
    // Read adblock config from file
    let config_path = &state.dns_dhcp_config_path;
    let content = match tokio::fs::read_to_string(config_path).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": format!("Config read error: {}", e)})),
    };

    let config: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => return Json(json!({"success": false, "error": format!("Config parse error: {}", e)})),
    };

    let adblock_config: hr_adblock::config::AdblockConfig = match config
        .get("adblock")
        .map(|v| serde_json::from_value(v.clone()))
    {
        Some(Ok(c)) => c,
        _ => hr_adblock::config::AdblockConfig::default(),
    };

    // Download and update
    let (domains, results) = hr_adblock::sources::download_all(&adblock_config.sources).await;
    let count = domains.len();

    // Save cache
    let cache_path = std::path::PathBuf::from(&adblock_config.data_dir).join("domains.json");
    let _ = hr_adblock::sources::save_cache(&domains, &cache_path);

    // Apply to engine
    {
        let mut engine = state.adblock.write().await;
        engine.set_blocked(domains);
        engine.set_whitelist(adblock_config.whitelist);
    }

    let source_results: Vec<Value> = results
        .iter()
        .map(|r| json!({"name": r.name, "domains": r.domain_count}))
        .collect();

    Json(json!({
        "success": true,
        "total_domains": count,
        "sources": source_results
    }))
}

/// Read adblock sources from config file for frontend display.
async fn read_adblock_sources(state: &ApiState) -> Vec<Value> {
    let config_path = &state.dns_dhcp_config_path;
    let content = match tokio::fs::read_to_string(config_path).await {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let config: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    config
        .get("adblock")
        .and_then(|a| a.get("sources"))
        .and_then(|s| s.as_array())
        .map(|sources| {
            sources
                .iter()
                .map(|s| {
                    json!({
                        "name": s.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        "url": s.get("url").and_then(|v| v.as_str()).unwrap_or("")
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

async fn search(
    State(state): State<ApiState>,
    Query(query): Query<SearchQuery>,
) -> Json<Value> {
    let q = query.q.unwrap_or_default();
    if q.is_empty() {
        return Json(json!({"success": true, "results": [], "query": ""}));
    }

    let engine = state.adblock.read().await;
    let results = engine.search(&q, 50);
    let is_blocked = engine.is_blocked(&q);

    Json(json!({
        "success": true,
        "query": q,
        "is_blocked": is_blocked,
        "results": results
    }))
}
