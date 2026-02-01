use axum::{
    extract::State,
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};

use crate::state::ApiState;

/// Legacy DNS-only routes (compat with old dnsmasq-era frontend).
/// Most functionality is in /api/dns-dhcp.
pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/cache-stats", get(cache_stats))
        .route("/status", get(status))
}

async fn cache_stats(State(state): State<ApiState>) -> Json<Value> {
    let dns = state.dns.read().await;
    let cache_size = dns.dns_cache.len().await;
    Json(json!({
        "success": true,
        "cache_size": cache_size,
        "adblock_enabled": dns.adblock_enabled
    }))
}

async fn status(State(state): State<ApiState>) -> Json<Value> {
    let dns = state.dns.read().await;
    Json(json!({
        "success": true,
        "active": true,
        "port": dns.config.port,
        "upstream_servers": dns.config.upstream_servers,
        "cache_size": dns.config.cache_size,
        "local_domain": dns.config.local_domain,
        "adblock_enabled": dns.adblock_enabled
    }))
}
