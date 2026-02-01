use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/status", get(status))
        .route("/routes", get(routes))
        .route("/reload", post(reload))
}

async fn status(State(state): State<ApiState>) -> Json<Value> {
    let config = state.proxy.config();
    Json(json!({
        "success": true,
        "active": true,
        "https_port": config.https_port,
        "http_port": config.http_port,
        "base_domain": config.base_domain,
        "tls_mode": config.tls_mode,
        "route_count": config.routes.len(),
        "active_routes": config.active_routes().len()
    }))
}

async fn routes(State(state): State<ApiState>) -> Json<Value> {
    let config = state.proxy.config();
    let routes: Vec<Value> = config
        .routes
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "domain": r.domain,
                "target_host": r.target_host,
                "target_port": r.target_port,
                "local_only": r.local_only,
                "require_auth": r.require_auth,
                "enabled": r.enabled
            })
        })
        .collect();

    Json(json!({"success": true, "routes": routes}))
}

async fn reload(State(state): State<ApiState>) -> Json<Value> {
    let proxy_config_path = &state.proxy_config_path;
    match hr_proxy::ProxyConfig::load_from_file(proxy_config_path) {
        Ok(new_config) => {
            state.proxy.reload_config(new_config);
            Json(json!({"success": true}))
        }
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}
