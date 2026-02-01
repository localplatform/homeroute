use serde_json::{json, Value};
use tracing::{error, info, warn};

const SCHEDULES_FILE: &str = "/data/wol-schedules.json";
const SERVERS_FILE: &str = "/data/servers.json";
const SSH_KEY_PATH: &str = "/data/ssh/id_rsa";

/// Run the WoL/power schedule executor.
/// Checks every 30 seconds for schedules that should be executed based on their cron expressions.
pub async fn run_scheduler() {
    info!("WoL scheduler started");

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;

        if let Err(e) = check_and_execute_schedules().await {
            error!("Scheduler error: {}", e);
        }
    }
}

async fn check_and_execute_schedules() -> Result<(), String> {
    let content = match tokio::fs::read_to_string(SCHEDULES_FILE).await {
        Ok(c) => c,
        Err(_) => return Ok(()), // No schedules file yet
    };

    let mut data: Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    let schedules = match data.get_mut("schedules").and_then(|s| s.as_array_mut()) {
        Some(s) => s,
        None => return Ok(()),
    };

    let now = chrono::Utc::now();
    let mut any_executed = false;

    for schedule in schedules.iter_mut() {
        let enabled = schedule
            .get("enabled")
            .and_then(|e| e.as_bool())
            .unwrap_or(false);
        if !enabled {
            continue;
        }

        let cron_expr = match schedule.get("cron").and_then(|c| c.as_str()) {
            Some(c) => c.to_string(),
            None => continue,
        };

        // Check if this schedule should run now
        if !should_run_now(&cron_expr, schedule, &now) {
            continue;
        }

        let server_id = schedule
            .get("serverId")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let action = schedule
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .to_string();
        let desc = schedule
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("unnamed")
            .to_string();

        info!("Executing schedule: {} (action: {}, server: {})", desc, action, server_id);

        match execute_action(&server_id, &action).await {
            Ok(()) => {
                info!("Schedule executed successfully: {}", desc);
            }
            Err(e) => {
                error!("Schedule execution failed: {} - {}", desc, e);
            }
        }

        schedule["lastRun"] = json!(now.to_rfc3339());
        any_executed = true;
    }

    if any_executed {
        let content = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
        let tmp = format!("{}.tmp", SCHEDULES_FILE);
        tokio::fs::write(&tmp, &content)
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::rename(&tmp, SCHEDULES_FILE)
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Check if a cron schedule should run now, based on the cron expression and last run time.
/// We check every 30s, so we match on the current minute and ensure we haven't already
/// run this minute.
fn should_run_now(cron_expr: &str, schedule: &Value, now: &chrono::DateTime<chrono::Utc>) -> bool {
    // Parse last run to avoid double-execution within the same minute
    if let Some(last_run_str) = schedule.get("lastRun").and_then(|l| l.as_str()) {
        if let Ok(last_run) = chrono::DateTime::parse_from_rfc3339(last_run_str) {
            let last_run = last_run.with_timezone(&chrono::Utc);
            // If we already ran this minute, skip
            if last_run.format("%Y-%m-%d %H:%M").to_string()
                == now.format("%Y-%m-%d %H:%M").to_string()
            {
                return false;
            }
        }
    }

    // Parse cron expression: "minute hour dom month dow"
    // Support standard 5-field cron
    let fields: Vec<&str> = cron_expr.trim().split_whitespace().collect();
    if fields.len() != 5 {
        warn!("Invalid cron expression: {}", cron_expr);
        return false;
    }

    let minute = now.format("%M").to_string().parse::<u32>().unwrap_or(0);
    let hour = now.format("%H").to_string().parse::<u32>().unwrap_or(0);
    let dom = now.format("%d").to_string().parse::<u32>().unwrap_or(1);
    let month = now.format("%m").to_string().parse::<u32>().unwrap_or(1);
    let dow = now.format("%u").to_string().parse::<u32>().unwrap_or(1); // 1=Monday

    cron_field_matches(fields[0], minute, 0, 59)
        && cron_field_matches(fields[1], hour, 0, 23)
        && cron_field_matches(fields[2], dom, 1, 31)
        && cron_field_matches(fields[3], month, 1, 12)
        && cron_field_matches(fields[4], dow % 7, 0, 6) // 0=Sunday in cron
}

/// Match a single cron field against a value. Supports: *, */n, n, n-m, n,m,o
fn cron_field_matches(field: &str, value: u32, _min: u32, _max: u32) -> bool {
    if field == "*" {
        return true;
    }

    // Handle */n (step)
    if let Some(step_str) = field.strip_prefix("*/") {
        if let Ok(step) = step_str.parse::<u32>() {
            return step > 0 && value % step == 0;
        }
        return false;
    }

    // Handle comma-separated values
    for part in field.split(',') {
        // Handle range n-m
        if let Some((start_str, end_str)) = part.split_once('-') {
            if let (Ok(start), Ok(end)) = (start_str.parse::<u32>(), end_str.parse::<u32>()) {
                if value >= start && value <= end {
                    return true;
                }
            }
        } else if let Ok(exact) = part.parse::<u32>() {
            if value == exact {
                return true;
            }
        }
    }

    false
}

async fn execute_action(server_id: &str, action: &str) -> Result<(), String> {
    match action {
        "wake" => {
            let mac = get_server_mac(server_id).await?;
            send_wol(&mac).await
        }
        "shutdown" => {
            ssh_power_command(server_id, "poweroff || shutdown -h now").await
        }
        "reboot" => {
            ssh_power_command(server_id, "reboot").await
        }
        _ => Err(format!("Unknown action: {}", action)),
    }
}

async fn get_server_mac(server_id: &str) -> Result<String, String> {
    let content = tokio::fs::read_to_string(SERVERS_FILE)
        .await
        .map_err(|e| e.to_string())?;
    let data: Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    let server = data
        .get("servers")
        .and_then(|s| s.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|s| s.get("id").and_then(|i| i.as_str()) == Some(server_id))
        })
        .ok_or("Server not found")?;

    server
        .get("mac")
        .and_then(|m| m.as_str())
        .map(String::from)
        .ok_or_else(|| "No MAC address configured".to_string())
}

async fn send_wol(mac: &str) -> Result<(), String> {
    let mac_bytes: Vec<u8> = mac
        .split(':')
        .filter_map(|b| u8::from_str_radix(b, 16).ok())
        .collect();

    if mac_bytes.len() != 6 {
        return Err("Invalid MAC address".to_string());
    }

    let mut packet = vec![0xFFu8; 6];
    for _ in 0..16 {
        packet.extend_from_slice(&mac_bytes);
    }

    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| e.to_string())?;
    socket.set_broadcast(true).map_err(|e| e.to_string())?;
    socket
        .send_to(&packet, "255.255.255.255:9")
        .await
        .map_err(|e| e.to_string())?;
    let _ = socket.send_to(&packet, "10.0.0.255:9").await;

    Ok(())
}

async fn ssh_power_command(server_id: &str, command: &str) -> Result<(), String> {
    let content = tokio::fs::read_to_string(SERVERS_FILE)
        .await
        .map_err(|e| e.to_string())?;
    let data: Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    let server = data
        .get("servers")
        .and_then(|s| s.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|s| s.get("id").and_then(|i| i.as_str()) == Some(server_id))
        })
        .ok_or("Server not found")?;

    let host = server.get("host").and_then(|h| h.as_str()).unwrap_or("");
    let port = server.get("port").and_then(|p| p.as_u64()).unwrap_or(22);

    let output = tokio::process::Command::new("ssh")
        .args([
            "-i",
            SSH_KEY_PATH,
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "ConnectTimeout=15",
            "-o",
            "BatchMode=yes",
            "-p",
            &port.to_string(),
            &format!("root@{}", host),
            command,
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    // Exit code 255 is expected for poweroff (connection drops)
    if output.status.success() || output.status.code() == Some(255) {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("SSH failed: {}", stderr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron_wildcard() {
        assert!(cron_field_matches("*", 0, 0, 59));
        assert!(cron_field_matches("*", 30, 0, 59));
    }

    #[test]
    fn test_cron_exact() {
        assert!(cron_field_matches("30", 30, 0, 59));
        assert!(!cron_field_matches("30", 31, 0, 59));
    }

    #[test]
    fn test_cron_step() {
        assert!(cron_field_matches("*/5", 0, 0, 59));
        assert!(cron_field_matches("*/5", 15, 0, 59));
        assert!(!cron_field_matches("*/5", 13, 0, 59));
    }

    #[test]
    fn test_cron_range() {
        assert!(cron_field_matches("1-5", 3, 1, 5));
        assert!(!cron_field_matches("1-5", 6, 1, 5));
    }

    #[test]
    fn test_cron_list() {
        assert!(cron_field_matches("1,3,5", 3, 0, 59));
        assert!(!cron_field_matches("1,3,5", 4, 0, 59));
    }
}
