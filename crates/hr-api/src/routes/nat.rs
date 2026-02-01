use axum::{
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/rules", get(nat_rules))
        .route("/filter", get(filter_rules))
        .route("/masquerade", get(masquerade_rules))
        .route("/forwards", get(port_forwards))
        .route("/status", get(firewall_status))
        .route("/routing-rules", get(routing_rules))
        .route("/stats", get(chain_stats))
}

async fn nat_rules() -> Json<Value> {
    match run_iptables(&["-t", "nat", "-L", "-n", "-v", "--line-numbers"]).await {
        Ok(output) => {
            let rules = parse_iptables_output(&output);
            Json(json!({"success": true, "rules": rules}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn filter_rules() -> Json<Value> {
    match run_iptables(&["-L", "-n", "-v", "--line-numbers"]).await {
        Ok(output) => {
            let rules = parse_iptables_output(&output);
            Json(json!({"success": true, "rules": rules}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn masquerade_rules() -> Json<Value> {
    match run_iptables(&["-t", "nat", "-L", "POSTROUTING", "-n", "-v", "--line-numbers"]).await {
        Ok(output) => {
            let rules = parse_chain_rules(&output);
            Json(json!({"success": true, "rules": rules}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn port_forwards() -> Json<Value> {
    match run_iptables(&["-t", "nat", "-L", "PREROUTING", "-n", "-v", "--line-numbers"]).await {
        Ok(output) => {
            let rules = parse_chain_rules(&output);
            // Filter for DNAT rules
            let dnat_rules: Vec<&Value> = rules
                .iter()
                .filter(|r| {
                    r.get("target")
                        .and_then(|t| t.as_str())
                        .is_some_and(|t| t == "DNAT")
                })
                .collect();
            Json(json!({"success": true, "forwards": dnat_rules}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn firewall_status() -> Json<Value> {
    // Check if nftables is available
    let nft = tokio::process::Command::new("nft")
        .args(["list", "tables"])
        .output()
        .await;

    let framework = if nft.is_ok() && nft.unwrap().status.success() {
        "nftables"
    } else {
        "iptables"
    };

    let active = run_iptables(&["-L", "-n"]).await.is_ok();

    Json(json!({
        "success": true,
        "active": active,
        "framework": framework
    }))
}

async fn routing_rules() -> Json<Value> {
    let output = tokio::process::Command::new("ip")
        .args(["rule", "show"])
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let rules: Vec<Value> = stdout
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| json!({"rule": l.trim()}))
                .collect();
            Json(json!({"success": true, "rules": rules}))
        }
        _ => Json(json!({"success": false, "error": "Failed to get routing rules"})),
    }
}

async fn chain_stats() -> Json<Value> {
    match run_iptables(&["-t", "nat", "-L", "-n", "-v", "-x"]).await {
        Ok(output) => {
            let stats = parse_chain_stats(&output);
            Json(json!({"success": true, "stats": stats}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn run_iptables(args: &[&str]) -> Result<String, String> {
    let output = tokio::process::Command::new("iptables")
        .args(args)
        .output()
        .await
        .map_err(|e| format!("Failed to execute iptables: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("iptables failed: {}", stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_iptables_output(output: &str) -> Value {
    let mut chains = serde_json::Map::new();
    let mut current_chain = String::new();
    let mut current_rules = Vec::new();
    let mut current_policy = String::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            if !current_chain.is_empty() {
                chains.insert(
                    current_chain.clone(),
                    json!({"policy": current_policy, "rules": current_rules}),
                );
                current_rules = Vec::new();
            }
            continue;
        }

        if line.starts_with("Chain ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                current_chain = parts[1].to_string();
                current_policy = parts
                    .get(3)
                    .map(|p| p.trim_end_matches(')').to_string())
                    .unwrap_or_default();
            }
        } else if !line.starts_with("num") && !line.starts_with("pkts") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 8 {
                current_rules.push(json!({
                    "num": parts.first().unwrap_or(&""),
                    "pkts": parts.get(1).unwrap_or(&""),
                    "bytes": parts.get(2).unwrap_or(&""),
                    "target": parts.get(3).unwrap_or(&""),
                    "prot": parts.get(4).unwrap_or(&""),
                    "opt": parts.get(5).unwrap_or(&""),
                    "source": parts.get(6).unwrap_or(&""),
                    "destination": parts.get(7).unwrap_or(&""),
                    "extra": parts.get(8..).map(|s| s.join(" ")).unwrap_or_default()
                }));
            }
        }
    }

    if !current_chain.is_empty() {
        chains.insert(
            current_chain,
            json!({"policy": current_policy, "rules": current_rules}),
        );
    }

    Value::Object(chains)
}

fn parse_chain_rules(output: &str) -> Vec<Value> {
    let mut rules = Vec::new();
    let mut in_rules = false;

    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("num") || line.starts_with("pkts") {
            in_rules = true;
            continue;
        }
        if line.is_empty() || line.starts_with("Chain ") {
            continue;
        }
        if in_rules {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 8 {
                rules.push(json!({
                    "num": parts[0],
                    "pkts": parts.get(1).unwrap_or(&""),
                    "bytes": parts.get(2).unwrap_or(&""),
                    "target": parts.get(3).unwrap_or(&""),
                    "prot": parts.get(4).unwrap_or(&""),
                    "opt": parts.get(5).unwrap_or(&""),
                    "source": parts.get(6).unwrap_or(&""),
                    "destination": parts.get(7).unwrap_or(&""),
                    "extra": parts.get(8..).map(|s| s.join(" ")).unwrap_or_default()
                }));
            }
        }
    }

    rules
}

fn parse_chain_stats(output: &str) -> Value {
    let mut stats = serde_json::Map::new();
    let mut current_chain = String::new();

    for line in output.lines() {
        if line.starts_with("Chain ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                current_chain = parts[1].to_string();
            }
        } else if !line.trim().is_empty()
            && !line.trim().starts_with("pkts")
            && !current_chain.is_empty()
        {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let pkts = parts[0].parse::<u64>().unwrap_or(0);
                let bytes = parts[1].parse::<u64>().unwrap_or(0);
                let entry = stats.entry(&current_chain).or_insert_with(|| {
                    json!({"total_packets": 0u64, "total_bytes": 0u64})
                });
                if let Some(obj) = entry.as_object_mut() {
                    let tp = obj.get("total_packets").and_then(|v| v.as_u64()).unwrap_or(0);
                    let tb = obj.get("total_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
                    obj.insert("total_packets".to_string(), json!(tp + pkts));
                    obj.insert("total_bytes".to_string(), json!(tb + bytes));
                }
            }
        }
    }

    Value::Object(stats)
}
