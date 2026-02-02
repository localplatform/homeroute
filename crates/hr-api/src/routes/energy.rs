use axum::{
    extract::Query,
    response::sse::{Event, Sse},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use tokio_stream::StreamExt;

use crate::state::ApiState;

const ENERGY_SCHEDULE_PATH: &str = "/var/lib/server-dashboard/energy-schedule.json";
const ENERGY_AUTOSELECT_PATH: &str = "/var/lib/server-dashboard/energy-autoselect.json";

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/cpu", get(cpu_info))
        .route("/status", get(governor_status))
        .route("/governor", post(set_governor))
        .route("/schedule", get(get_schedule).post(save_schedule))
        .route("/modes", get(list_modes))
        .route("/mode", get(current_mode))
        .route("/mode/{mode}", post(apply_mode))
        .route("/interfaces", get(energy_interfaces))
        .route("/autoselect", get(get_autoselect).post(save_autoselect))
        .route("/benchmark", get(benchmark_status))
        .route("/benchmark/start", post(start_benchmark))
        .route("/benchmark/stop", post(stop_benchmark))
        .route("/events", get(sse_events))
}

async fn cpu_info() -> Json<Value> {
    let temp = read_cpu_temperature().await;
    let freq = read_cpu_frequency().await;
    let usage = read_cpu_usage().await;
    let model = read_cpu_model().await;

    Json(json!({
        "success": true,
        "temperature": temp,
        "frequency": freq,
        "usage": usage,
        "model": model
    }))
}

async fn read_cpu_temperature() -> Option<f64> {
    // Search hwmon for CPU temp (k10temp, coretemp, zenpower)
    let hwmon_dir = "/sys/class/hwmon";
    let mut entries = match tokio::fs::read_dir(hwmon_dir).await {
        Ok(e) => e,
        Err(_) => return None,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name_path = entry.path().join("name");
        if let Ok(name) = tokio::fs::read_to_string(&name_path).await {
            let name = name.trim();
            if name == "k10temp" || name == "coretemp" || name == "zenpower" {
                let temp_path = entry.path().join("temp1_input");
                if let Ok(val) = tokio::fs::read_to_string(&temp_path).await {
                    if let Ok(millideg) = val.trim().parse::<f64>() {
                        return Some(millideg / 1000.0);
                    }
                }
            }
        }
    }
    None
}

async fn read_cpu_frequency() -> Value {
    let mut freqs = Vec::new();
    let mut min_freq = None;
    let mut max_freq = None;

    for i in 0..128 {
        let cur = format!("/sys/devices/system/cpu/cpu{}/cpufreq/scaling_cur_freq", i);
        match tokio::fs::read_to_string(&cur).await {
            Ok(val) => {
                if let Ok(khz) = val.trim().parse::<u64>() {
                    freqs.push(khz);
                }
            }
            Err(_) => break,
        }

        if i == 0 {
            let min_path = format!("/sys/devices/system/cpu/cpu{}/cpufreq/scaling_min_freq", i);
            let max_path = format!("/sys/devices/system/cpu/cpu{}/cpufreq/scaling_max_freq", i);
            min_freq = tokio::fs::read_to_string(&min_path)
                .await
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok());
            max_freq = tokio::fs::read_to_string(&max_path)
                .await
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok());
        }
    }

    let avg = if freqs.is_empty() {
        0
    } else {
        freqs.iter().sum::<u64>() / freqs.len() as u64
    };

    let current_ghz = avg as f64 / 1_000_000.0;
    let min_ghz = min_freq.map(|f| f as f64 / 1_000_000.0);
    let max_ghz = max_freq.map(|f| f as f64 / 1_000_000.0);

    json!({
        "current": current_ghz,
        "min": min_ghz,
        "max": max_ghz,
        "cores": freqs.len()
    })
}

async fn read_cpu_usage() -> Option<f64> {
    // Read /proc/stat twice with a small delay
    let stat1 = tokio::fs::read_to_string("/proc/stat").await.ok()?;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let stat2 = tokio::fs::read_to_string("/proc/stat").await.ok()?;

    fn parse_cpu_line(line: &str) -> Option<(u64, u64)> {
        let parts: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse().ok())
            .collect();
        if parts.len() < 4 {
            return None;
        }
        let idle = parts[3];
        let total: u64 = parts.iter().sum();
        Some((idle, total))
    }

    let line1 = stat1.lines().next()?;
    let line2 = stat2.lines().next()?;
    let (idle1, total1) = parse_cpu_line(line1)?;
    let (idle2, total2) = parse_cpu_line(line2)?;

    let idle_delta = idle2.saturating_sub(idle1) as f64;
    let total_delta = total2.saturating_sub(total1) as f64;
    if total_delta == 0.0 {
        return Some(0.0);
    }

    Some(((total_delta - idle_delta) / total_delta) * 100.0)
}

async fn read_cpu_model() -> String {
    if let Ok(content) = tokio::fs::read_to_string("/proc/cpuinfo").await {
        for line in content.lines() {
            if line.starts_with("model name") {
                if let Some((_k, v)) = line.split_once(':') {
                    return v.trim().to_string();
                }
            }
        }
    }
    "Unknown".to_string()
}

async fn governor_status() -> Json<Value> {
    let current = tokio::fs::read_to_string(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
    )
    .await
    .map(|s| s.trim().to_string())
    .unwrap_or_default();

    let available = tokio::fs::read_to_string(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors",
    )
    .await
    .map(|s| s.trim().split_whitespace().map(String::from).collect::<Vec<_>>())
    .unwrap_or_default();

    Json(json!({
        "success": true,
        "current": current,
        "available": available
    }))
}

#[derive(Deserialize)]
struct GovernorRequest {
    governor: String,
}

async fn set_governor(Json(body): Json<GovernorRequest>) -> Json<Value> {
    // Set governor for all CPU cores
    for i in 0..128 {
        let path = format!(
            "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_governor",
            i
        );
        if tokio::fs::metadata(&path).await.is_err() {
            break;
        }
        if let Err(e) = tokio::fs::write(&path, &body.governor).await {
            return Json(json!({"success": false, "error": format!("Failed to set governor for cpu{}: {}", i, e)}));
        }
    }

    Json(json!({"success": true, "governor": body.governor}))
}

async fn get_schedule() -> Json<Value> {
    let default_config = json!({"enabled": false, "nightStart": "00:00", "nightEnd": "08:00"});
    match tokio::fs::read_to_string(ENERGY_SCHEDULE_PATH).await {
        Ok(content) => match serde_json::from_str::<Value>(&content) {
            Ok(config) => Json(json!({"success": true, "config": config})),
            Err(_) => Json(json!({"success": true, "config": default_config})),
        },
        Err(_) => Json(json!({"success": true, "config": default_config})),
    }
}

async fn save_schedule(Json(body): Json<Value>) -> Json<Value> {
    match serde_json::to_string_pretty(&body) {
        Ok(content) => {
            if let Err(e) = tokio::fs::write(ENERGY_SCHEDULE_PATH, &content).await {
                return Json(json!({"success": false, "error": e.to_string()}));
            }
            Json(json!({"success": true}))
        }
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn list_modes() -> Json<Value> {
    Json(json!({
        "success": true,
        "modes": {
            "economy": {"governor": "powersave", "epp": "power", "maxFreqPercent": 60},
            "auto": {"governor": "powersave", "epp": "balance_power", "maxFreqPercent": 85},
            "performance": {"governor": "performance", "epp": "performance", "maxFreqPercent": 100}
        }
    }))
}

async fn current_mode() -> Json<Value> {
    let governor = tokio::fs::read_to_string(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
    )
    .await
    .map(|s| s.trim().to_string())
    .unwrap_or_default();

    let mode = match governor.as_str() {
        "performance" => "performance",
        "powersave" => "economy", // Could be auto, but we guess economy
        _ => "unknown",
    };

    Json(json!({"success": true, "mode": mode, "governor": governor}))
}

#[derive(Deserialize)]
struct ApplyModeParams {
    mode: String,
}

async fn apply_mode(axum::extract::Path(mode): axum::extract::Path<String>) -> Json<Value> {
    let (governor, epp, max_pct) = match mode.as_str() {
        "economy" => ("powersave", "power", 60u32),
        "auto" => ("powersave", "balance_power", 85),
        "performance" => ("performance", "performance", 100),
        _ => return Json(json!({"success": false, "error": "Mode inconnu"})),
    };

    // Set governor
    for i in 0..128 {
        let gov_path = format!(
            "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_governor",
            i
        );
        if tokio::fs::metadata(&gov_path).await.is_err() {
            break;
        }
        let _ = tokio::fs::write(&gov_path, governor).await;

        // Set EPP if available
        let epp_path = format!(
            "/sys/devices/system/cpu/cpu{}/cpufreq/energy_performance_preference",
            i
        );
        if tokio::fs::metadata(&epp_path).await.is_ok() {
            let _ = tokio::fs::write(&epp_path, epp).await;
        }

        // Set max frequency
        let max_path = format!(
            "/sys/devices/system/cpu/cpu{}/cpufreq/cpuinfo_max_freq",
            i
        );
        if let Ok(max_str) = tokio::fs::read_to_string(&max_path).await {
            if let Ok(max_khz) = max_str.trim().parse::<u64>() {
                let target = max_khz * max_pct as u64 / 100;
                let scaling_max = format!(
                    "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_max_freq",
                    i
                );
                let _ = tokio::fs::write(&scaling_max, target.to_string()).await;
            }
        }
    }

    Json(json!({"success": true, "mode": mode}))
}

async fn energy_interfaces() -> Json<Value> {
    // List network interfaces with IP info for energy auto-select
    let output = tokio::process::Command::new("ip")
        .args(["-j", "addr", "show"])
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if let Ok(ifaces) = serde_json::from_str::<Vec<Value>>(&stdout) {
                let result: Vec<Value> = ifaces
                    .iter()
                    .filter(|i| {
                        i.get("ifname")
                            .and_then(|n| n.as_str())
                            .is_some_and(|n| !n.starts_with("lo") && !n.starts_with("veth"))
                    })
                    .map(|i| {
                        let name = i.get("ifname").and_then(|n| n.as_str()).unwrap_or("");
                        let flags = i.get("flags").and_then(|f| f.as_array());
                        let state = if flags.is_some_and(|f| f.iter().any(|v| v.as_str() == Some("UP"))) {
                            "UP"
                        } else {
                            "DOWN"
                        };
                        // Find first IPv4 address
                        let primary_ip = i
                            .get("addr_info")
                            .and_then(|a| a.as_array())
                            .and_then(|addrs| {
                                addrs.iter().find_map(|a| {
                                    if a.get("family").and_then(|f| f.as_str()) == Some("inet") {
                                        a.get("local").and_then(|l| l.as_str()).map(String::from)
                                    } else {
                                        None
                                    }
                                })
                            })
                            .unwrap_or_default();
                        json!({"name": name, "primaryIp": primary_ip, "state": state})
                    })
                    .collect();
                return Json(json!({"success": true, "interfaces": result}));
            }
            Json(json!({"success": true, "interfaces": []}))
        }
        _ => Json(json!({"success": false, "error": "Failed to list interfaces"})),
    }
}

async fn get_autoselect() -> Json<Value> {
    let default_config = json!({
        "enabled": false,
        "networkInterface": null,
        "thresholds": {"low": 1000, "high": 10000},
        "averagingTime": 3,
        "sampleInterval": 1000
    });
    match tokio::fs::read_to_string(ENERGY_AUTOSELECT_PATH).await {
        Ok(content) => match serde_json::from_str::<Value>(&content) {
            Ok(config) => Json(json!({"success": true, "config": config})),
            Err(_) => Json(json!({"success": true, "config": default_config})),
        },
        Err(_) => Json(json!({"success": true, "config": default_config})),
    }
}

async fn save_autoselect(Json(body): Json<Value>) -> Json<Value> {
    match serde_json::to_string_pretty(&body) {
        Ok(content) => {
            if let Err(e) = tokio::fs::write(ENERGY_AUTOSELECT_PATH, &content).await {
                return Json(json!({"success": false, "error": e.to_string()}));
            }
            Json(json!({"success": true}))
        }
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn benchmark_status() -> Json<Value> {
    // Check if stress-ng or yes is running
    let output = tokio::process::Command::new("pgrep")
        .args(["-f", "stress-ng|yes"])
        .output()
        .await;

    let running = output
        .map(|o| o.status.success())
        .unwrap_or(false);

    Json(json!({"success": true, "running": running}))
}

#[derive(Deserialize)]
struct BenchmarkQuery {
    #[serde(default = "default_duration")]
    duration: u64,
}

fn default_duration() -> u64 {
    60
}

async fn start_benchmark(Query(query): Query<BenchmarkQuery>) -> Json<Value> {
    let duration = query.duration.min(600); // Max 10 minutes

    // Try stress-ng first, fall back to yes
    let result = tokio::process::Command::new("stress-ng")
        .args(["--cpu", "0", "--timeout", &format!("{}s", duration)])
        .spawn();

    match result {
        Ok(_child) => Json(json!({"success": true, "tool": "stress-ng", "duration": duration})),
        Err(_) => {
            // Fallback: spawn yes | head for each CPU core
            let num_cpus = num_cpus().await;
            for _ in 0..num_cpus {
                let _ = tokio::process::Command::new("sh")
                    .args(["-c", &format!("timeout {} yes > /dev/null", duration)])
                    .spawn();
            }
            Json(json!({"success": true, "tool": "yes", "duration": duration, "cores": num_cpus}))
        }
    }
}

async fn num_cpus() -> usize {
    if let Ok(content) = tokio::fs::read_to_string("/proc/cpuinfo").await {
        content.lines().filter(|l| l.starts_with("processor")).count()
    } else {
        1
    }
}

async fn stop_benchmark() -> Json<Value> {
    let _ = tokio::process::Command::new("pkill")
        .args(["-f", "stress-ng"])
        .output()
        .await;
    let _ = tokio::process::Command::new("pkill")
        .args(["-f", "yes"])
        .output()
        .await;

    Json(json!({"success": true}))
}

/// SSE endpoint for real-time energy events.
/// Sends periodic keepalive comments to maintain the connection.
async fn sse_events() -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let interval = tokio::time::interval(std::time::Duration::from_secs(15));
    let stream = tokio_stream::wrappers::IntervalStream::new(interval)
        .map(|_| Ok(Event::default().comment("keepalive")));
    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keepalive"),
    )
}
