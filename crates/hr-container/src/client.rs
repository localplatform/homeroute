use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::process::Command;
use tracing::{info, warn};

const NSPAWN_UNIT_DIR: &str = "/etc/systemd/nspawn";
const DEFAULT_STORAGE: &str = "/var/lib/machines";

/// Information about a systemd-nspawn container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NspawnContainerInfo {
    pub name: String,
    pub status: String,
    pub storage_path: String,
}

/// Client for managing systemd-nspawn containers via `machinectl` CLI.
pub struct NspawnClient;

impl NspawnClient {
    /// Create and start a container: debootstrap rootfs, write .nspawn unit,
    /// write network config, machinectl start, wait for readiness.
    pub async fn create_container(name: &str, storage_path: &Path) -> Result<()> {
        info!(container = name, storage = %storage_path.display(), "Creating nspawn container");

        // Bootstrap Ubuntu rootfs
        crate::rootfs::bootstrap_ubuntu(name, storage_path).await?;

        // Create workspace directory (must exist before start due to Bind= in .nspawn unit)
        Self::create_workspace(name, storage_path).await?;

        // Write .nspawn unit
        Self::write_nspawn_unit(name, storage_path, "bridge:br-lan").await?;

        // Write network config inside rootfs
        Self::write_network_config(name, storage_path).await?;

        // Start the container
        Self::start_container(name).await?;

        // Wait for the container to be ready
        Self::wait_ready(name).await?;

        info!(container = name, "Nspawn container created and running");
        Ok(())
    }

    /// Wait for a container to be running and have a network interface up.
    async fn wait_ready(name: &str) -> Result<()> {
        for i in 0..30 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            let output = Command::new("machinectl")
                .args(["shell", name, "/bin/bash", "-c", "ip link show host0"])
                .output()
                .await;

            if let Ok(output) = output {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    if stdout.contains("UP") {
                        return Ok(());
                    }
                }
            }

            if i == 29 {
                warn!(container = name, "Container network not ready after 30s, proceeding anyway");
            }
        }
        Ok(())
    }

    /// Push a local file into a container by copying directly into the rootfs.
    pub async fn push_file(container: &str, src: &Path, dest: &str, storage_path: &Path) -> Result<()> {
        let target = storage_path.join(container).join(dest);

        // Ensure parent directory exists
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await
                .with_context(|| format!("failed to create parent dir {}", parent.display()))?;
        }

        tokio::fs::copy(src, &target).await
            .with_context(|| format!("failed to copy {} to {}", src.display(), target.display()))?;

        Ok(())
    }

    /// Execute a command inside a container and return stdout.
    pub async fn exec(container: &str, cmd: &[&str]) -> Result<String> {
        let joined = cmd.join(" ");
        let output = Command::new("machinectl")
            .args(["shell", container, "/bin/bash", "-c", &joined])
            .output()
            .await
            .context("failed to run machinectl shell")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("machinectl shell {cmd:?} failed: {stderr}");
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Execute a command inside a container with retries.
    pub async fn exec_with_retry(container: &str, cmd: &[&str], max_retries: u32) -> Result<String> {
        let mut last_error = None;
        for attempt in 0..max_retries {
            match Self::exec(container, cmd).await {
                Ok(output) => return Ok(output),
                Err(e) => {
                    last_error = Some(e);
                    if attempt + 1 < max_retries {
                        warn!(
                            container,
                            attempt = attempt + 1,
                            max_retries,
                            "Command failed, retrying in 3s..."
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    }
                }
            }
        }
        Err(last_error.unwrap())
    }

    /// Wait for network connectivity inside a container (DNS resolution working).
    pub async fn wait_for_network(container: &str, timeout_secs: u32) -> Result<()> {
        for i in 0..timeout_secs {
            let result = Self::exec(container, &["getent", "hosts", "archive.ubuntu.com"]).await;
            if result.is_ok() {
                info!(container, elapsed_secs = i + 1, "Network connectivity confirmed");
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        warn!(container, timeout_secs, "Network connectivity not confirmed after timeout, proceeding anyway");
        Ok(())
    }

    /// Create the workspace directory and bind-mount configuration.
    pub async fn create_workspace(container: &str, storage_path: &Path) -> Result<()> {
        let ws_dir = storage_path.join(format!("{container}-workspace"));
        tokio::fs::create_dir_all(&ws_dir).await
            .with_context(|| format!("failed to create workspace dir {}", ws_dir.display()))?;

        info!(container, workspace = %ws_dir.display(), "Workspace directory created");
        Ok(())
    }

    /// Stop and delete a container: machinectl terminate + rm -rf rootfs + workspace + .nspawn unit.
    pub async fn delete_container(name: &str, storage_path: &Path) -> Result<()> {
        info!(container = name, "Deleting nspawn container");

        // Force stop (ignore error if already stopped)
        let _ = Command::new("machinectl")
            .args(["terminate", name])
            .output()
            .await;

        // Wait briefly for termination
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Remove rootfs
        let rootfs = storage_path.join(name);
        if rootfs.exists() {
            tokio::fs::remove_dir_all(&rootfs).await
                .with_context(|| format!("failed to remove rootfs {}", rootfs.display()))?;
        }

        // Remove workspace
        let ws_dir = storage_path.join(format!("{name}-workspace"));
        if ws_dir.exists() {
            tokio::fs::remove_dir_all(&ws_dir).await
                .with_context(|| format!("failed to remove workspace {}", ws_dir.display()))?;
        }

        // Remove .nspawn unit
        let unit_path = format!("{NSPAWN_UNIT_DIR}/{name}.nspawn");
        let _ = tokio::fs::remove_file(&unit_path).await;

        info!(container = name, "Nspawn container deleted");
        Ok(())
    }

    /// List containers filtered by `hr-v2-` prefix.
    pub async fn list_containers() -> Result<Vec<NspawnContainerInfo>> {
        let output = Command::new("machinectl")
            .args(["list", "--no-legend", "--no-pager"])
            .output()
            .await
            .context("failed to run machinectl list")?;

        if !output.status.success() {
            anyhow::bail!("machinectl list failed");
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut containers = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            let name = parts[0];
            if !name.starts_with("hr-v2-") {
                continue;
            }

            let status = parts.get(2).unwrap_or(&"unknown").to_string();

            containers.push(NspawnContainerInfo {
                name: name.to_string(),
                status,
                storage_path: DEFAULT_STORAGE.to_string(),
            });
        }

        Ok(containers)
    }

    /// Start a container.
    pub async fn start_container(name: &str) -> Result<()> {
        info!(container = name, "Starting nspawn container");

        let output = Command::new("machinectl")
            .args(["start", name])
            .output()
            .await
            .context("failed to run machinectl start")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("machinectl start {name} failed: {stderr}");
        }

        Ok(())
    }

    /// Stop a container.
    pub async fn stop_container(name: &str) -> Result<()> {
        info!(container = name, "Stopping nspawn container");

        let output = Command::new("machinectl")
            .args(["terminate", name])
            .output()
            .await
            .context("failed to run machinectl terminate")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Don't fail if already stopped
            if !stderr.contains("not running") && !stderr.contains("not known") {
                anyhow::bail!("machinectl terminate {name} failed: {stderr}");
            }
        }

        Ok(())
    }

    /// Write the .nspawn unit file for a container.
    /// `network_mode` is either "bridge:br-lan" or "macvlan:enp7s0f0".
    pub async fn write_nspawn_unit(name: &str, storage_path: &Path, network_mode: &str) -> Result<()> {
        tokio::fs::create_dir_all(NSPAWN_UNIT_DIR).await
            .context("failed to create nspawn unit directory")?;

        let ws_path = storage_path.join(format!("{name}-workspace"));
        let network_line = if let Some(iface) = network_mode.strip_prefix("macvlan:") {
            format!("MACVLAN={iface}")
        } else if let Some(bridge) = network_mode.strip_prefix("bridge:") {
            format!("Bridge={bridge}")
        } else {
            format!("Bridge={network_mode}")
        };

        // If storage_path is not the default, add Directory= to [Exec]
        let directory_line = if storage_path != Path::new(DEFAULT_STORAGE) {
            let rootfs = storage_path.join(name);
            format!("\nDirectory={}", rootfs.display())
        } else {
            String::new()
        };

        let content = format!(
            "[Exec]\n\
             Boot=yes\n\
             PrivateUsers=no{directory_line}\n\
             \n\
             [Network]\n\
             {network_line}\n\
             \n\
             [Files]\n\
             Bind={}:/root/workspace\n",
            ws_path.display()
        );

        let unit_path = format!("{NSPAWN_UNIT_DIR}/{name}.nspawn");
        tokio::fs::write(&unit_path, &content).await
            .with_context(|| format!("failed to write nspawn unit {unit_path}"))?;

        info!(container = name, unit = unit_path, "Nspawn unit written");
        Ok(())
    }

    /// Write network configuration inside the container rootfs.
    /// Sets up systemd-networkd for DHCP on host0 and resolv.conf pointing to HomeRoute DNS.
    pub async fn write_network_config(name: &str, storage_path: &Path) -> Result<()> {
        let rootfs = storage_path.join(name);

        // Write systemd-networkd config for host0
        let network_dir = rootfs.join("etc/systemd/network");
        tokio::fs::create_dir_all(&network_dir).await
            .context("failed to create network config dir")?;

        let network_config = "[Match]\n\
             Name=host0\n\
             \n\
             [Network]\n\
             DHCP=yes\n\
             \n\
             [DHCPv4]\n\
             UseHostname=false\n";

        tokio::fs::write(network_dir.join("80-container.network"), network_config).await
            .context("failed to write network config")?;

        // Write resolv.conf pointing to HomeRoute DNS
        let resolv_path = rootfs.join("etc/resolv.conf");
        // Remove symlink if present
        let _ = tokio::fs::remove_file(&resolv_path).await;
        tokio::fs::write(&resolv_path, "nameserver 10.0.0.254\n").await
            .context("failed to write resolv.conf")?;

        info!(container = name, "Network config written in rootfs");
        Ok(())
    }
}
