use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use hr_common::events::UpdateEvent;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::broadcast;
use tracing::error;

use crate::state::ApiState;

static CHECK_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static UPGRADE_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/status", get(check_status))
        .route("/last", get(last_check))
        .route("/check", post(run_check))
        .route("/cancel", post(cancel_check))
        .route("/upgrade/status", get(upgrade_status))
        .route("/upgrade/apt", post(upgrade_apt))
        .route("/upgrade/apt-full", post(upgrade_apt_full))
        .route("/upgrade/snap", post(upgrade_snap))
        .route("/upgrade/cancel", post(cancel_upgrade))
}

const LAST_CHECK_PATH: &str = "/var/lib/server-dashboard/last-update-check.json";

async fn check_status() -> Json<Value> {
    let running = CHECK_RUNNING.load(std::sync::atomic::Ordering::Relaxed);
    Json(json!({"success": true, "running": running}))
}

async fn last_check() -> Json<Value> {
    match tokio::fs::read_to_string(LAST_CHECK_PATH).await {
        Ok(content) => match serde_json::from_str::<Value>(&content) {
            Ok(result) => Json(json!({"success": true, "result": result})),
            Err(_) => Json(json!({"success": true, "result": null})),
        },
        Err(_) => Json(json!({"success": true, "result": null})),
    }
}

async fn run_check(State(state): State<ApiState>) -> Json<Value> {
    if CHECK_RUNNING.load(std::sync::atomic::Ordering::Relaxed) {
        return Json(json!({"success": false, "error": "Verification deja en cours"}));
    }

    CHECK_RUNNING.store(true, std::sync::atomic::Ordering::Relaxed);
    let tx = state.events.updates.clone();

    // Spawn the check in background so it streams events via WebSocket
    tokio::spawn(async move {
        let _ = tx.send(UpdateEvent::Started);
        let start = std::time::Instant::now();

        // Phase 1: apt update
        let _ = tx.send(UpdateEvent::Phase {
            phase: "apt-update".to_string(),
            message: "Mise a jour des listes de paquets...".to_string(),
        });

        stream_command(&tx, "apt", &["update", "-q"]).await;

        // Phase 2: apt list --upgradable
        let _ = tx.send(UpdateEvent::Phase {
            phase: "apt-check".to_string(),
            message: "Verification des paquets APT...".to_string(),
        });

        let apt_output = tokio::process::Command::new("apt")
            .args(["list", "--upgradable"])
            .output()
            .await;

        let apt_packages = match apt_output {
            Ok(o) => parse_apt_list(&String::from_utf8_lossy(&o.stdout)),
            Err(_) => vec![],
        };

        let security_count = apt_packages
            .iter()
            .filter(|p| {
                p.get("isSecurity")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false)
            })
            .count();

        let _ = tx.send(UpdateEvent::AptComplete {
            packages: apt_packages.clone(),
            security_count,
        });

        // Phase 3: snap refresh --list
        let _ = tx.send(UpdateEvent::Phase {
            phase: "snap-check".to_string(),
            message: "Verification des snaps...".to_string(),
        });

        let snap_output = tokio::process::Command::new("snap")
            .args(["refresh", "--list"])
            .output()
            .await;

        let snap_packages = match snap_output {
            Ok(o) if o.status.success() => {
                parse_snap_list(&String::from_utf8_lossy(&o.stdout))
            }
            _ => vec![],
        };

        let _ = tx.send(UpdateEvent::SnapComplete {
            snaps: snap_packages.clone(),
        });

        // Phase 4: needrestart
        let _ = tx.send(UpdateEvent::Phase {
            phase: "needrestart".to_string(),
            message: "Verification des services...".to_string(),
        });

        let needrestart = tokio::process::Command::new("needrestart")
            .args(["-b"])
            .output()
            .await;

        let needrestart_info = match needrestart {
            Ok(o) => parse_needrestart(&String::from_utf8_lossy(&o.stdout)),
            Err(_) => json!({"kernelRebootNeeded": false, "services": []}),
        };

        let _ = tx.send(UpdateEvent::NeedrestartComplete(needrestart_info.clone()));

        let duration = start.elapsed().as_millis() as u64;

        let summary = json!({
            "totalUpdates": apt_packages.len() + snap_packages.len(),
            "securityUpdates": security_count,
            "servicesNeedingRestart": needrestart_info.get("services").and_then(|s| s.as_array()).map(|a| a.len()).unwrap_or(0)
        });

        let result = json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "duration": duration,
            "apt": { "packages": apt_packages, "securityCount": security_count },
            "snap": { "packages": snap_packages },
            "needrestart": needrestart_info,
            "summary": summary
        });

        if let Ok(content) = serde_json::to_string_pretty(&result) {
            let _ = tokio::fs::write(LAST_CHECK_PATH, &content).await;
        }

        let _ = tx.send(UpdateEvent::Complete {
            success: true,
            summary,
            duration,
        });

        CHECK_RUNNING.store(false, std::sync::atomic::Ordering::Relaxed);
    });

    Json(json!({"success": true, "message": "Verification lancee"}))
}

async fn cancel_check(State(state): State<ApiState>) -> Json<Value> {
    let _ = tokio::process::Command::new("pkill")
        .args(["-f", "apt update"])
        .output()
        .await;
    CHECK_RUNNING.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = state.events.updates.send(UpdateEvent::Cancelled);
    Json(json!({"success": true}))
}

async fn upgrade_status() -> Json<Value> {
    let running = UPGRADE_RUNNING.load(std::sync::atomic::Ordering::Relaxed);
    Json(json!({"success": true, "running": running}))
}

async fn upgrade_apt(State(state): State<ApiState>) -> Json<Value> {
    run_upgrade(state, "apt", &["upgrade", "-y"]).await
}

async fn upgrade_apt_full(State(state): State<ApiState>) -> Json<Value> {
    run_upgrade(state, "apt", &["full-upgrade", "-y"]).await
}

async fn upgrade_snap(State(state): State<ApiState>) -> Json<Value> {
    run_upgrade(state, "snap", &["refresh"]).await
}

async fn run_upgrade(state: ApiState, cmd: &str, args: &[&str]) -> Json<Value> {
    if UPGRADE_RUNNING.load(std::sync::atomic::Ordering::Relaxed) {
        return Json(json!({"success": false, "error": "Mise a jour deja en cours"}));
    }

    UPGRADE_RUNNING.store(true, std::sync::atomic::Ordering::Relaxed);
    let tx = state.events.updates.clone();
    let cmd = cmd.to_string();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();

    tokio::spawn(async move {
        let upgrade_type = if cmd == "snap" {
            "snap".to_string()
        } else if args.contains(&"full-upgrade".to_string()) {
            "apt-full".to_string()
        } else {
            "apt".to_string()
        };

        let _ = tx.send(UpdateEvent::UpgradeStarted {
            upgrade_type: upgrade_type.clone(),
        });

        let start = std::time::Instant::now();

        let mut child = match tokio::process::Command::new(&cmd)
            .args(&args)
            .env("DEBIAN_FRONTEND", "noninteractive")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(UpdateEvent::UpgradeComplete {
                    upgrade_type,
                    success: false,
                    duration: 0,
                    error: Some(e.to_string()),
                });
                UPGRADE_RUNNING.store(false, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        };

        // Stream stdout
        if let Some(stdout) = child.stdout.take() {
            let tx_c = tx.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx_c.send(UpdateEvent::UpgradeOutput { line });
                }
            });
        }

        // Stream stderr
        if let Some(stderr) = child.stderr.take() {
            let tx_c = tx.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx_c.send(UpdateEvent::UpgradeOutput { line });
                }
            });
        }

        let status = child.wait().await;
        let duration = start.elapsed().as_millis() as u64;
        let success = status.map(|s| s.success()).unwrap_or(false);

        let _ = tx.send(UpdateEvent::UpgradeComplete {
            upgrade_type,
            success,
            duration,
            error: if success {
                None
            } else {
                Some("Upgrade failed".to_string())
            },
        });

        UPGRADE_RUNNING.store(false, std::sync::atomic::Ordering::Relaxed);
    });

    Json(json!({"success": true, "message": "Mise a jour lancee"}))
}

async fn cancel_upgrade(State(state): State<ApiState>) -> Json<Value> {
    let _ = tokio::process::Command::new("pkill")
        .args(["-f", "apt upgrade|apt full-upgrade|snap refresh"])
        .output()
        .await;
    UPGRADE_RUNNING.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = state.events.updates.send(UpdateEvent::UpgradeCancelled);
    Json(json!({"success": true}))
}

/// Stream command output line by line as UpdateEvent::Output
async fn stream_command(tx: &broadcast::Sender<UpdateEvent>, cmd: &str, args: &[&str]) {
    let mut child = match tokio::process::Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to spawn {}: {}", cmd, e);
            return;
        }
    };

    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx.send(UpdateEvent::Output { line });
        }
    }

    let _ = child.wait().await;
}

fn parse_apt_list(output: &str) -> Vec<Value> {
    output
        .lines()
        .filter(|line| line.contains('/'))
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() {
                return None;
            }
            let name_source: Vec<&str> = parts[0].split('/').collect();
            let name = name_source.first()?.to_string();
            let source = name_source.get(1).unwrap_or(&"").to_string();
            let new_version = parts.get(1).unwrap_or(&"").to_string();
            let current_version = if line.contains("upgradable from:") {
                line.split("upgradable from: ")
                    .nth(1)
                    .map(|v| v.trim_end_matches(']').to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let is_security = source.contains("security");

            Some(json!({
                "name": name,
                "currentVersion": current_version,
                "newVersion": new_version,
                "isSecurity": is_security
            }))
        })
        .collect()
}

fn parse_snap_list(output: &str) -> Vec<Value> {
    output
        .lines()
        .skip(1)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                return None;
            }
            Some(json!({
                "name": parts[0],
                "newVersion": parts.get(1).unwrap_or(&""),
                "publisher": parts.get(4).unwrap_or(&"")
            }))
        })
        .collect()
}

fn parse_needrestart(output: &str) -> Value {
    let kernel_reboot = output.contains("NEEDRESTART-KSTA: 3");
    let services: Vec<String> = output
        .lines()
        .filter(|l| l.starts_with("NEEDRESTART-SVC:"))
        .filter_map(|l| l.split(':').nth(1).map(|s| s.trim().to_string()))
        .collect();

    json!({
        "kernelRebootNeeded": kernel_reboot,
        "services": services
    })
}
