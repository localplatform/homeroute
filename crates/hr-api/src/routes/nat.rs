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
            // Rename "out" -> "outInterface" for frontend
            let transformed: Vec<Value> = rules
                .into_iter()
                .map(|mut r| {
                    if let Some(obj) = r.as_object_mut() {
                        if let Some(out_val) = obj.remove("out") {
                            obj.insert("outInterface".to_string(), out_val);
                        }
                    }
                    r
                })
                .collect();
            Json(json!({"success": true, "rules": transformed}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

async fn port_forwards() -> Json<Value> {
    match run_iptables(&["-t", "nat", "-L", "PREROUTING", "-n", "-v", "--line-numbers"]).await {
        Ok(output) => {
            let rules = parse_chain_rules(&output);
            // Filter for DNAT rules and transform to frontend format
            let dnat_rules: Vec<Value> = rules
                .into_iter()
                .filter(|r| {
                    r.get("target")
                        .and_then(|t| t.as_str())
                        .is_some_and(|t| t == "DNAT")
                })
                .map(|r| transform_dnat_rule(&r))
                .collect();
            Json(json!({"success": true, "rules": dnat_rules}))
        }
        Err(e) => Json(json!({"success": false, "error": e})),
    }
}

/// Transform a DNAT iptables rule into frontend-expected format.
/// Parses the "extra" field to extract destination port and forward target.
/// Example extra: "tcp dpt:80 to:192.168.1.100:8080"
fn transform_dnat_rule(rule: &Value) -> Value {
    let extra = rule.get("extra").and_then(|v| v.as_str()).unwrap_or("");
    let prot = rule.get("prot").and_then(|v| v.as_str()).unwrap_or("");
    let in_iface = rule.get("in").and_then(|v| v.as_str()).unwrap_or("*");

    // Extract destination port from "dpt:PORT" or "dpts:START:END"
    let dest_port = extra
        .split_whitespace()
        .find_map(|part| {
            part.strip_prefix("dpt:").or_else(|| part.strip_prefix("dpts:"))
        })
        .unwrap_or("")
        .to_string();

    // Extract forward target from "to:IP:PORT"
    let forward_to = extra
        .split_whitespace()
        .find_map(|part| part.strip_prefix("to:"))
        .unwrap_or("")
        .to_string();

    json!({
        "protocol": prot,
        "destinationPort": dest_port,
        "forwardTo": forward_to,
        "inInterface": if in_iface == "*" { "any" } else { in_iface },
        "pkts": rule.get("pkts").and_then(|v| v.as_str()).unwrap_or("0"),
        "bytes": rule.get("bytes").and_then(|v| v.as_str()).unwrap_or("0")
    })
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
        "status": {
            "active": active,
            "framework": framework
        }
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
                .map(|l| parse_routing_rule(l.trim()))
                .collect();
            Json(json!({"success": true, "rules": rules}))
        }
        _ => Json(json!({"success": false, "error": "Failed to get routing rules"})),
    }
}

/// Parse a single `ip rule show` line into structured fields.
/// Example: "5210:	from all fwmark 0x80000/0xff0000 lookup main"
fn parse_routing_rule(line: &str) -> Value {
    let (priority, rest) = line.split_once(':').unwrap_or(("", line));
    let rest = rest.trim();
    let parts: Vec<&str> = rest.split_whitespace().collect();

    let mut src = "all".to_string();
    let mut dst = "all".to_string();
    let mut table = String::new();
    let mut fwmark = String::new();

    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            "from" => {
                if let Some(v) = parts.get(i + 1) {
                    src = v.to_string();
                    i += 1;
                }
            }
            "to" => {
                if let Some(v) = parts.get(i + 1) {
                    dst = v.to_string();
                    i += 1;
                }
            }
            "lookup" | "table" => {
                if let Some(v) = parts.get(i + 1) {
                    table = v.to_string();
                    i += 1;
                }
            }
            "fwmark" => {
                if let Some(v) = parts.get(i + 1) {
                    fwmark = v.to_string();
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    json!({
        "priority": priority.trim(),
        "src": src,
        "dst": dst,
        "table": table,
        "fwmark": if fwmark.is_empty() { Value::Null } else { Value::String(fwmark) }
    })
}

async fn chain_stats() -> Json<Value> {
    match run_iptables(&["-L", "-n", "-v", "-x"]).await {
        Ok(output) => {
            let stats = parse_chain_stats(&output);
            Json(json!({"success": true, "stats": { "chains": stats }}))
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

/// Parse full iptables output (multiple chains) into a map of chain name → {policy, rules}.
/// iptables -v --line-numbers columns:
/// num  pkts  bytes  target  prot  opt  in  out  source  destination  [extra...]
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
            if let Some(rule) = parse_iptables_rule_line(line) {
                current_rules.push(rule);
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

/// Parse a single chain's rules (used for masquerade/forwards single-chain output).
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
            if let Some(rule) = parse_iptables_rule_line(line) {
                rules.push(rule);
            }
        }
    }

    rules
}

/// Parse a single iptables -v --line-numbers rule line.
/// Columns: num  pkts  bytes  target  prot  opt  in  out  source  destination  [extra...]
fn parse_iptables_rule_line(line: &str) -> Option<Value> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 10 {
        return None;
    }
    Some(json!({
        "num": parts[0],
        "pkts": parts[1],
        "bytes": parts[2],
        "target": parts[3],
        "prot": parts[4],
        "opt": parts[5],
        "in": parts[6],
        "out": parts[7],
        "source": parts[8],
        "destination": parts[9],
        "extra": parts.get(10..).map(|s| s.join(" ")).unwrap_or_default()
    }))
}

/// Parse chain stats, returning { "CHAIN_NAME": { "packets": N, "bytes": N } }.
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
                    json!({"packets": 0u64, "bytes": 0u64})
                });
                if let Some(obj) = entry.as_object_mut() {
                    let tp = obj.get("packets").and_then(|v| v.as_u64()).unwrap_or(0);
                    let tb = obj.get("bytes").and_then(|v| v.as_u64()).unwrap_or(0);
                    obj.insert("packets".to_string(), json!(tp + pkts));
                    obj.insert("bytes".to_string(), json!(tb + bytes));
                }
            }
        }
    }

    Value::Object(stats)
}
