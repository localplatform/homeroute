use axum::{
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/interfaces", get(interfaces))
        .route("/routes", get(ipv4_routes))
        .route("/routes6", get(ipv6_routes))
}

async fn interfaces() -> Json<Value> {
    match run_json_command("ip", &["-j", "addr", "show"]).await {
        Ok(raw) => {
            // Filter out veth interfaces and transform to frontend format
            let filtered: Vec<Value> = raw
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter(|iface| {
                    iface
                        .get("ifname")
                        .and_then(|n| n.as_str())
                        .is_some_and(|name| !name.starts_with("veth"))
                })
                .map(|iface| transform_interface(iface))
                .collect();
            Json(json!({"success": true, "interfaces": filtered}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

/// Transform raw `ip -j addr show` entry to frontend-expected format.
fn transform_interface(raw: &Value) -> Value {
    let flags = raw.get("flags").and_then(|f| f.as_array());
    let state = if flags.is_some_and(|f| f.iter().any(|v| v.as_str() == Some("UP"))) {
        "UP"
    } else {
        "DOWN"
    };

    let addresses: Vec<Value> = raw
        .get("addr_info")
        .and_then(|a| a.as_array())
        .unwrap_or(&vec![])
        .iter()
        .map(|addr| {
            json!({
                "address": addr.get("local").and_then(|v| v.as_str()).unwrap_or(""),
                "family": addr.get("family").and_then(|v| v.as_str()).unwrap_or(""),
                "prefixlen": addr.get("prefixlen"),
                "scope": addr.get("scope").and_then(|v| v.as_str()).unwrap_or("")
            })
        })
        .collect();

    json!({
        "name": raw.get("ifname").and_then(|v| v.as_str()).unwrap_or(""),
        "state": state,
        "mac": raw.get("address").and_then(|v| v.as_str()).unwrap_or(""),
        "mtu": raw.get("mtu"),
        "addresses": addresses
    })
}

async fn ipv4_routes() -> Json<Value> {
    match run_json_command("ip", &["-j", "route", "show"]).await {
        Ok(raw) => {
            let routes: Vec<Value> = raw
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|r| {
                    json!({
                        "destination": r.get("dst").and_then(|v| v.as_str()).unwrap_or(""),
                        "gateway": r.get("gateway").and_then(|v| v.as_str()),
                        "device": r.get("dev").and_then(|v| v.as_str()).unwrap_or(""),
                        "metric": r.get("metric")
                    })
                })
                .collect();
            Json(json!({"success": true, "routes": routes}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn ipv6_routes() -> Json<Value> {
    match run_json_command("ip", &["-j", "-6", "route", "show"]).await {
        Ok(raw) => {
            let routes: Vec<Value> = raw
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|r| {
                    json!({
                        "destination": r.get("dst").and_then(|v| v.as_str()).unwrap_or(""),
                        "gateway": r.get("gateway").and_then(|v| v.as_str()),
                        "device": r.get("dev").and_then(|v| v.as_str()).unwrap_or(""),
                        "metric": r.get("metric")
                    })
                })
                .collect();
            Json(json!({"success": true, "routes": routes}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn run_json_command(cmd: &str, args: &[&str]) -> Result<Value, String> {
    let output = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("Failed to execute {}: {}", cmd, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{} failed: {}", cmd, stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse output: {}", e))
}
