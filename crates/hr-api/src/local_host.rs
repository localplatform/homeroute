//! LocalHostAdapter — wraps local LXD operations for container management.

use tracing::error;

/// Run an LXC command locally with a timeout.
async fn lxc_cmd(args: &[&str], timeout_secs: u64) -> Result<std::process::Output, String> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        tokio::process::Command::new("lxc").args(args).output(),
    ).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(format!("lxc error: {e}")),
        Err(_) => Err(format!("lxc {} timed out after {timeout_secs}s", args.first().unwrap_or(&"?"))),
    }
}

pub struct LocalHostAdapter;

impl LocalHostAdapter {
    /// Delete container + workspace volume.
    pub async fn delete_container(container_name: &str) -> Result<(), String> {
        if let Err(e) = hr_lxd::LxdClient::delete_container(container_name).await {
            error!(container = %container_name, "Failed to delete source container: {e}. Manual cleanup required.");
            return Err(format!("Failed to delete container: {e}"));
        }
        // Also delete workspace volume
        let vol_name = format!("{container_name}-workspace");
        let _ = lxc_cmd(&["storage", "volume", "delete", "default", &vol_name], 30).await;
        Ok(())
    }

    /// Stop container (force).
    pub async fn stop_container(container_name: &str) -> Result<(), String> {
        let output = lxc_cmd(&["stop", container_name, "--force"], 60).await
            .map_err(|e| format!("Failed to stop container: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to stop container: {}", stderr));
        }
        Ok(())
    }

    /// Start container.
    pub async fn start_container(container_name: &str) -> Result<(), String> {
        let output = lxc_cmd(&["start", container_name], 60).await
            .map_err(|e| format!("Failed to start container: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to start container: {}", stderr));
        }
        Ok(())
    }
}
