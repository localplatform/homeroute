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
            // Filter out veth interfaces
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
                .cloned()
                .collect();
            Json(json!({"success": true, "interfaces": filtered}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn ipv4_routes() -> Json<Value> {
    match run_json_command("ip", &["-j", "route", "show"]).await {
        Ok(routes) => Json(json!({"success": true, "routes": routes})),
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn ipv6_routes() -> Json<Value> {
    match run_json_command("ip", &["-j", "-6", "route", "show"]).await {
        Ok(routes) => Json(json!({"success": true, "routes": routes})),
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
