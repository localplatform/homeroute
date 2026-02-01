use axum::{
    extract::{Path, State},
    routing::{get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/config", get(get_config))
        .route("/config/domain", put(update_domain))
        .route("/hosts", get(list_hosts).post(add_host))
        .route("/hosts/{id}", put(update_host).delete(delete_host))
        .route("/hosts/{id}/toggle", post(toggle_host))
        .route("/status", get(proxy_status))
        .route("/reload", post(reload_proxy))
        .route("/certificates/status", get(certificates_status))
        .route("/certificates/renew", post(renew_certificates))
        .route("/environments", get(list_environments).post(add_environment))
        .route(
            "/environments/{id}",
            put(update_environment).delete(delete_environment),
        )
        .route(
            "/applications",
            get(list_applications).post(add_application),
        )
        .route(
            "/applications/{id}",
            put(update_application).delete(delete_application),
        )
        .route("/applications/{id}/toggle", post(toggle_application))
}

/// Load the reverseproxy-config.json
async fn load_rp_config(state: &ApiState) -> Result<Value, String> {
    let content = tokio::fs::read_to_string(&state.reverseproxy_config_path)
        .await
        .map_err(|e| format!("Read error: {}", e))?;
    serde_json::from_str(&content).map_err(|e| format!("Parse error: {}", e))
}

/// Save the reverseproxy-config.json
async fn save_rp_config(state: &ApiState, config: &Value) -> Result<(), String> {
    let content =
        serde_json::to_string_pretty(config).map_err(|e| format!("Serialize error: {}", e))?;
    let tmp = state.reverseproxy_config_path.with_extension("json.tmp");
    tokio::fs::write(&tmp, &content)
        .await
        .map_err(|e| format!("Write error: {}", e))?;
    tokio::fs::rename(&tmp, &state.reverseproxy_config_path)
        .await
        .map_err(|e| format!("Rename error: {}", e))?;
    Ok(())
}

/// Sync all routes to rust-proxy-config.json and reload proxy
async fn sync_and_reload(state: &ApiState) -> Result<(), String> {
    let rp_config = load_rp_config(state).await?;
    let base_domain = rp_config
        .get("baseDomain")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string();

    let hosts = rp_config
        .get("hosts")
        .and_then(|h| h.as_array())
        .cloned()
        .unwrap_or_default();

    let applications = rp_config
        .get("applications")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();

    let environments = rp_config
        .get("environments")
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();

    // Build proxy routes
    let mut routes = Vec::new();

    // Add host-based routes
    for host in &hosts {
        if host.get("enabled").and_then(|e| e.as_bool()) != Some(true) {
            continue;
        }
        let domain = if let Some(custom) = host.get("customDomain").and_then(|d| d.as_str()) {
            if !custom.is_empty() {
                custom.to_string()
            } else if let Some(sub) = host.get("subdomain").and_then(|s| s.as_str()) {
                format!("{}.{}", sub, base_domain)
            } else {
                continue;
            }
        } else if let Some(sub) = host.get("subdomain").and_then(|s| s.as_str()) {
            format!("{}.{}", sub, base_domain)
        } else {
            continue;
        };

        routes.push(json!({
            "id": host.get("id").unwrap_or(&json!("")),
            "domain": domain,
            "backend": "rust",
            "target_host": host.get("targetHost").unwrap_or(&json!("localhost")),
            "target_port": host.get("targetPort").unwrap_or(&json!(80)),
            "local_only": host.get("localOnly").unwrap_or(&json!(false)),
            "require_auth": host.get("requireAuth").unwrap_or(&json!(false)),
            "enabled": true
        }));
    }

    // Add application-based routes
    for app in &applications {
        if app.get("enabled").and_then(|e| e.as_bool()) != Some(true) {
            continue;
        }
        let slug = app.get("slug").and_then(|s| s.as_str()).unwrap_or("");
        let endpoints = app.get("endpoints").and_then(|e| e.as_object());

        if let Some(endpoints) = endpoints {
            for (env_id, env_endpoints) in endpoints {
                let prefix = environments
                    .iter()
                    .find(|e| e.get("id").and_then(|i| i.as_str()) == Some(env_id))
                    .and_then(|e| e.get("prefix").and_then(|p| p.as_str()))
                    .unwrap_or(env_id);

                // Frontend endpoint
                if let Some(fe) = env_endpoints.get("frontend") {
                    let domain = if prefix.is_empty() {
                        format!("{}.{}", slug, base_domain)
                    } else {
                        format!("{}.{}.{}", slug, prefix, base_domain)
                    };
                    routes.push(json!({
                        "id": format!("{}-{}-fe", app.get("id").and_then(|i| i.as_str()).unwrap_or(""), env_id),
                        "domain": domain,
                        "backend": "rust",
                        "target_host": fe.get("targetHost").unwrap_or(&json!("localhost")),
                        "target_port": fe.get("targetPort").unwrap_or(&json!(80)),
                        "local_only": fe.get("localOnly").unwrap_or(&json!(false)),
                        "require_auth": fe.get("requireAuth").unwrap_or(&json!(false)),
                        "enabled": true
                    }));
                }

                // API endpoints
                if let Some(apis) = env_endpoints.get("apis").and_then(|a| a.as_array()) {
                    for api in apis {
                        let api_slug = api.get("slug").and_then(|s| s.as_str()).unwrap_or("api");
                        let domain = if prefix.is_empty() {
                            format!("{}-{}.{}", slug, api_slug, base_domain)
                        } else {
                            format!("{}-{}.{}.{}", slug, api_slug, prefix, base_domain)
                        };
                        routes.push(json!({
                            "id": format!("{}-{}-{}", app.get("id").and_then(|i| i.as_str()).unwrap_or(""), env_id, api_slug),
                            "domain": domain,
                            "backend": "rust",
                            "target_host": api.get("targetHost").unwrap_or(&json!("localhost")),
                            "target_port": api.get("targetPort").unwrap_or(&json!(80)),
                            "local_only": api.get("localOnly").unwrap_or(&json!(false)),
                            "require_auth": api.get("requireAuth").unwrap_or(&json!(false)),
                            "enabled": true
                        }));
                    }
                }
            }
        }
    }

    // Load current proxy config, update routes, save
    let proxy_config_path = &state.proxy_config_path;
    let proxy_content = tokio::fs::read_to_string(proxy_config_path)
        .await
        .unwrap_or_else(|_| "{}".to_string());
    let mut proxy_config: Value =
        serde_json::from_str(&proxy_content).unwrap_or_else(|_| json!({}));

    proxy_config["routes"] = json!(routes);
    proxy_config["base_domain"] = json!(base_domain);

    let content =
        serde_json::to_string_pretty(&proxy_config).map_err(|e| format!("Serialize: {}", e))?;
    let tmp = proxy_config_path.with_extension("json.tmp");
    tokio::fs::write(&tmp, &content)
        .await
        .map_err(|e| format!("Write: {}", e))?;
    tokio::fs::rename(&tmp, proxy_config_path)
        .await
        .map_err(|e| format!("Rename: {}", e))?;

    // Reload proxy config in memory
    if let Ok(new_proxy_config) =
        hr_proxy::ProxyConfig::load_from_file(proxy_config_path)
    {
        state.proxy.reload_config(new_proxy_config);
    }

    Ok(())
}

async fn get_config(State(state): State<ApiState>) -> Json<Value> {
    match load_rp_config(&state).await {
        Ok(config) => Json(json!({"success": true, "config": config})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

#[derive(Deserialize)]
struct UpdateDomainRequest {
    domain: String,
}

async fn update_domain(
    State(state): State<ApiState>,
    Json(body): Json<UpdateDomainRequest>,
) -> Json<Value> {
    let mut config = match load_rp_config(&state).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    config["baseDomain"] = json!(body.domain);

    if let Err(e) = save_rp_config(&state, &config).await {
        return Json(json!({"success": false, "error": e}));
    }

    if let Err(e) = sync_and_reload(&state).await {
        return Json(json!({"success": false, "error": format!("Sync failed: {}", e)}));
    }

    Json(json!({"success": true}))
}

async fn list_hosts(State(state): State<ApiState>) -> Json<Value> {
    match load_rp_config(&state).await {
        Ok(config) => {
            let hosts = config.get("hosts").cloned().unwrap_or(json!([]));
            Json(json!({"success": true, "hosts": hosts}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn add_host(State(state): State<ApiState>, Json(body): Json<Value>) -> Json<Value> {
    let mut config = match load_rp_config(&state).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    let id = uuid::Uuid::new_v4().to_string();
    let mut host = body;
    host["id"] = json!(id);
    host["createdAt"] = json!(chrono::Utc::now().to_rfc3339());
    if host.get("enabled").is_none() {
        host["enabled"] = json!(true);
    }

    let hosts = config
        .get_mut("hosts")
        .and_then(|h| h.as_array_mut());
    match hosts {
        Some(arr) => arr.push(host.clone()),
        None => config["hosts"] = json!([host]),
    }

    if let Err(e) = save_rp_config(&state, &config).await {
        return Json(json!({"success": false, "error": e}));
    }
    if let Err(e) = sync_and_reload(&state).await {
        return Json(json!({"success": false, "error": format!("Sync failed: {}", e)}));
    }

    Json(json!({"success": true, "host": host}))
}

async fn update_host(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(updates): Json<Value>,
) -> Json<Value> {
    let mut config = match load_rp_config(&state).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    let hosts = config.get_mut("hosts").and_then(|h| h.as_array_mut());
    if let Some(hosts) = hosts {
        if let Some(host) = hosts.iter_mut().find(|h| h.get("id").and_then(|i| i.as_str()) == Some(&id)) {
            if let Some(obj) = updates.as_object() {
                for (k, v) in obj {
                    host[k] = v.clone();
                }
            }
            host["updatedAt"] = json!(chrono::Utc::now().to_rfc3339());
        } else {
            return Json(json!({"success": false, "error": "Host non trouve"}));
        }
    }

    if let Err(e) = save_rp_config(&state, &config).await {
        return Json(json!({"success": false, "error": e}));
    }
    if let Err(e) = sync_and_reload(&state).await {
        return Json(json!({"success": false, "error": format!("Sync failed: {}", e)}));
    }

    Json(json!({"success": true}))
}

async fn delete_host(State(state): State<ApiState>, Path(id): Path<String>) -> Json<Value> {
    let mut config = match load_rp_config(&state).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    if let Some(hosts) = config.get_mut("hosts").and_then(|h| h.as_array_mut()) {
        hosts.retain(|h| h.get("id").and_then(|i| i.as_str()) != Some(&id));
    }

    if let Err(e) = save_rp_config(&state, &config).await {
        return Json(json!({"success": false, "error": e}));
    }
    if let Err(e) = sync_and_reload(&state).await {
        return Json(json!({"success": false, "error": format!("Sync failed: {}", e)}));
    }

    Json(json!({"success": true}))
}

async fn toggle_host(State(state): State<ApiState>, Path(id): Path<String>) -> Json<Value> {
    let mut config = match load_rp_config(&state).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    if let Some(hosts) = config.get_mut("hosts").and_then(|h| h.as_array_mut()) {
        if let Some(host) = hosts.iter_mut().find(|h| h.get("id").and_then(|i| i.as_str()) == Some(&id)) {
            let current = host.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true);
            host["enabled"] = json!(!current);
        }
    }

    if let Err(e) = save_rp_config(&state, &config).await {
        return Json(json!({"success": false, "error": e}));
    }
    if let Err(e) = sync_and_reload(&state).await {
        return Json(json!({"success": false, "error": format!("Sync failed: {}", e)}));
    }

    Json(json!({"success": true}))
}

async fn proxy_status(State(state): State<ApiState>) -> Json<Value> {
    let config = state.proxy.config();
    let route_count = config.routes.len();
    let active_count = config.active_routes().len();

    Json(json!({
        "success": true,
        "active": true,
        "routes": route_count,
        "active_routes": active_count,
        "base_domain": config.base_domain
    }))
}

async fn reload_proxy(State(state): State<ApiState>) -> Json<Value> {
    match sync_and_reload(&state).await {
        Ok(()) => Json(json!({"success": true})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn certificates_status(State(state): State<ApiState>) -> Json<Value> {
    match state.ca.list_certificates() {
        Ok(certs) => {
            let statuses: Vec<Value> = certs
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "domains": c.domains,
                        "issued_at": c.issued_at.to_rfc3339(),
                        "expires_at": c.expires_at.to_rfc3339(),
                        "expired": c.is_expired(),
                        "needs_renewal": c.needs_renewal(30)
                    })
                })
                .collect();
            Json(json!({"success": true, "certificates": statuses}))
        }
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn renew_certificates(State(state): State<ApiState>) -> Json<Value> {
    let candidates = match state.ca.certificates_needing_renewal() {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e.to_string()})),
    };

    let mut renewed = Vec::new();
    let mut errors = Vec::new();

    for cert in candidates {
        match state.ca.renew_certificate(&cert.id).await {
            Ok(new_cert) => renewed.push(json!({"id": new_cert.id, "domains": new_cert.domains})),
            Err(e) => errors.push(json!({"id": cert.id, "error": e.to_string()})),
        }
    }

    Json(json!({
        "success": errors.is_empty(),
        "renewed": renewed,
        "errors": errors
    }))
}

async fn list_environments(State(state): State<ApiState>) -> Json<Value> {
    match load_rp_config(&state).await {
        Ok(config) => {
            let envs = config.get("environments").cloned().unwrap_or(json!([]));
            Json(json!({"success": true, "environments": envs}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn add_environment(State(state): State<ApiState>, Json(body): Json<Value>) -> Json<Value> {
    let mut config = match load_rp_config(&state).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    let id = uuid::Uuid::new_v4().to_string();
    let mut env = body;
    env["id"] = json!(id);

    let envs = config.get_mut("environments").and_then(|e| e.as_array_mut());
    match envs {
        Some(arr) => arr.push(env.clone()),
        None => config["environments"] = json!([env]),
    }

    if let Err(e) = save_rp_config(&state, &config).await {
        return Json(json!({"success": false, "error": e}));
    }

    Json(json!({"success": true, "environment": env}))
}

async fn update_environment(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(updates): Json<Value>,
) -> Json<Value> {
    let mut config = match load_rp_config(&state).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    if let Some(envs) = config.get_mut("environments").and_then(|e| e.as_array_mut()) {
        if let Some(env) = envs.iter_mut().find(|e| e.get("id").and_then(|i| i.as_str()) == Some(&id)) {
            if let Some(obj) = updates.as_object() {
                for (k, v) in obj {
                    env[k] = v.clone();
                }
            }
        }
    }

    if let Err(e) = save_rp_config(&state, &config).await {
        return Json(json!({"success": false, "error": e}));
    }
    if let Err(e) = sync_and_reload(&state).await {
        return Json(json!({"success": false, "error": format!("Sync failed: {}", e)}));
    }

    Json(json!({"success": true}))
}

async fn delete_environment(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let mut config = match load_rp_config(&state).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    if let Some(envs) = config.get_mut("environments").and_then(|e| e.as_array_mut()) {
        envs.retain(|e| e.get("id").and_then(|i| i.as_str()) != Some(&id));
    }

    if let Err(e) = save_rp_config(&state, &config).await {
        return Json(json!({"success": false, "error": e}));
    }

    Json(json!({"success": true}))
}

async fn list_applications(State(state): State<ApiState>) -> Json<Value> {
    match load_rp_config(&state).await {
        Ok(config) => {
            let apps = config.get("applications").cloned().unwrap_or(json!([]));
            Json(json!({"success": true, "applications": apps}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn add_application(State(state): State<ApiState>, Json(body): Json<Value>) -> Json<Value> {
    let mut config = match load_rp_config(&state).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    let id = uuid::Uuid::new_v4().to_string();
    let mut app = body;
    app["id"] = json!(id);
    app["createdAt"] = json!(chrono::Utc::now().to_rfc3339());
    if app.get("enabled").is_none() {
        app["enabled"] = json!(true);
    }

    let apps = config.get_mut("applications").and_then(|a| a.as_array_mut());
    match apps {
        Some(arr) => arr.push(app.clone()),
        None => config["applications"] = json!([app]),
    }

    if let Err(e) = save_rp_config(&state, &config).await {
        return Json(json!({"success": false, "error": e}));
    }
    if let Err(e) = sync_and_reload(&state).await {
        return Json(json!({"success": false, "error": format!("Sync failed: {}", e)}));
    }

    Json(json!({"success": true, "application": app}))
}

async fn update_application(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(updates): Json<Value>,
) -> Json<Value> {
    let mut config = match load_rp_config(&state).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    if let Some(apps) = config.get_mut("applications").and_then(|a| a.as_array_mut()) {
        if let Some(app) = apps.iter_mut().find(|a| a.get("id").and_then(|i| i.as_str()) == Some(&id)) {
            if let Some(obj) = updates.as_object() {
                for (k, v) in obj {
                    app[k] = v.clone();
                }
            }
            app["updatedAt"] = json!(chrono::Utc::now().to_rfc3339());
        }
    }

    if let Err(e) = save_rp_config(&state, &config).await {
        return Json(json!({"success": false, "error": e}));
    }
    if let Err(e) = sync_and_reload(&state).await {
        return Json(json!({"success": false, "error": format!("Sync failed: {}", e)}));
    }

    Json(json!({"success": true}))
}

async fn delete_application(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let mut config = match load_rp_config(&state).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    if let Some(apps) = config.get_mut("applications").and_then(|a| a.as_array_mut()) {
        apps.retain(|a| a.get("id").and_then(|i| i.as_str()) != Some(&id));
    }

    if let Err(e) = save_rp_config(&state, &config).await {
        return Json(json!({"success": false, "error": e}));
    }
    if let Err(e) = sync_and_reload(&state).await {
        return Json(json!({"success": false, "error": format!("Sync failed: {}", e)}));
    }

    Json(json!({"success": true}))
}

async fn toggle_application(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let mut config = match load_rp_config(&state).await {
        Ok(c) => c,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    if let Some(apps) = config.get_mut("applications").and_then(|a| a.as_array_mut()) {
        if let Some(app) = apps.iter_mut().find(|a| a.get("id").and_then(|i| i.as_str()) == Some(&id)) {
            let current = app.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true);
            app["enabled"] = json!(!current);
        }
    }

    if let Err(e) = save_rp_config(&state, &config).await {
        return Json(json!({"success": false, "error": e}));
    }
    if let Err(e) = sync_and_reload(&state).await {
        return Json(json!({"success": false, "error": format!("Sync failed: {}", e)}));
    }

    Json(json!({"success": true}))
}
