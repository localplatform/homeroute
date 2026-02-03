//! Systemd service management for powersave functionality.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, Result};
use hr_registry::protocol::{ServiceConfig, ServiceState, ServiceType};
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, info, warn};

/// Timeout for systemctl operations.
const SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for waiting for a service to be ready.
const SERVICE_READY_TIMEOUT: Duration = Duration::from_secs(60);

/// Manages systemd services within the container.
#[derive(Debug, Clone)]
pub struct ServiceManager {
    /// App service units (e.g., ["myapp.service"]).
    app_units: Vec<String>,
    /// Database service units (e.g., ["postgresql.service"]).
    db_units: Vec<String>,
}

impl ServiceManager {
    /// Create a new service manager with the given configuration.
    pub fn new(config: &ServiceConfig) -> Self {
        Self {
            app_units: config.app.clone(),
            db_units: config.db.clone(),
        }
    }

    /// Update the service configuration.
    pub fn update_config(&mut self, config: &ServiceConfig) {
        self.app_units = config.app.clone();
        self.db_units = config.db.clone();
    }

    /// Check if this service type is configured (has units to manage).
    pub fn is_configured(&self, service_type: ServiceType) -> bool {
        match service_type {
            ServiceType::CodeServer => true, // Always configured
            ServiceType::App => !self.app_units.is_empty(),
            ServiceType::Db => !self.db_units.is_empty(),
        }
    }

    /// Get the units for a service type.
    fn units_for(&self, service_type: ServiceType) -> Vec<&str> {
        match service_type {
            ServiceType::CodeServer => vec!["code-server.service"],
            ServiceType::App => self.app_units.iter().map(|s| s.as_str()).collect(),
            ServiceType::Db => self.db_units.iter().map(|s| s.as_str()).collect(),
        }
    }

    /// Get the current state of a service type.
    /// Returns Running if ALL units are running, Stopped if ALL are stopped,
    /// otherwise returns the "worst" state.
    pub async fn get_state(&self, service_type: ServiceType) -> ServiceState {
        let units = self.units_for(service_type);
        if units.is_empty() {
            return ServiceState::Stopped;
        }

        let mut all_running = true;
        let mut all_stopped = true;
        let mut any_starting = false;
        let mut any_stopping = false;

        for unit in units {
            match self.get_unit_state(unit).await {
                ServiceState::Running => all_stopped = false,
                ServiceState::Stopped | ServiceState::ManuallyOff => all_running = false,
                ServiceState::Starting => {
                    any_starting = true;
                    all_stopped = false;
                    all_running = false;
                }
                ServiceState::Stopping => {
                    any_stopping = true;
                    all_stopped = false;
                    all_running = false;
                }
            }
        }

        if any_starting {
            ServiceState::Starting
        } else if any_stopping {
            ServiceState::Stopping
        } else if all_running {
            ServiceState::Running
        } else if all_stopped {
            ServiceState::Stopped
        } else {
            // Mixed state - some running, some stopped
            ServiceState::Stopped
        }
    }

    /// Get the state of a single systemd unit.
    async fn get_unit_state(&self, unit: &str) -> ServiceState {
        // Check if active
        let output = Command::new("systemctl")
            .args(["is-active", unit])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await;

        let status = match output {
            Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
            Err(_) => return ServiceState::Stopped,
        };

        match status.as_str() {
            "active" => ServiceState::Running,
            "activating" => ServiceState::Starting,
            "deactivating" => ServiceState::Stopping,
            _ => ServiceState::Stopped,
        }
    }

    /// Check if a service type is running (all units active).
    pub async fn is_running(&self, service_type: ServiceType) -> bool {
        matches!(self.get_state(service_type).await, ServiceState::Running)
    }

    /// Start all units for a service type.
    pub async fn start(&self, service_type: ServiceType) -> Result<()> {
        let units = self.units_for(service_type);
        if units.is_empty() {
            return Ok(());
        }

        info!(service_type = ?service_type, units = ?units, "Starting services");

        for unit in units {
            self.start_unit(unit).await?;
        }

        Ok(())
    }

    /// Start a single systemd unit.
    async fn start_unit(&self, unit: &str) -> Result<()> {
        let result = timeout(
            SYSTEMCTL_TIMEOUT,
            Command::new("systemctl")
                .args(["start", unit])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) if output.status.success() => {
                debug!(unit, "Service started");
                Ok(())
            }
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(anyhow!("Failed to start {}: {}", unit, stderr))
            }
            Ok(Err(e)) => Err(anyhow!("Failed to execute systemctl: {}", e)),
            Err(_) => Err(anyhow!("Timeout starting {}", unit)),
        }
    }

    /// Stop all units for a service type.
    pub async fn stop(&self, service_type: ServiceType) -> Result<()> {
        let units = self.units_for(service_type);
        if units.is_empty() {
            return Ok(());
        }

        info!(service_type = ?service_type, units = ?units, "Stopping services");

        for unit in units {
            self.stop_unit(unit).await?;
        }

        // For code-server, kill any remaining child processes (extensions, LSP, etc.)
        if service_type == ServiceType::CodeServer {
            self.cleanup_code_server_processes().await;
        }

        Ok(())
    }

    /// Kill any remaining code-server related processes after stopping the service.
    /// This includes extension hosts, language servers, file watchers, etc.
    async fn cleanup_code_server_processes(&self) {
        // Kill all processes matching code-server's node binary
        let result = Command::new("pkill")
            .args(["-f", "code-server"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => {
                info!("Killed remaining code-server processes");
            }
            Ok(_) => {
                // pkill returns non-zero if no processes matched, which is fine
                debug!("No remaining code-server processes to kill");
            }
            Err(e) => {
                warn!(error = %e, "Failed to run pkill for code-server cleanup");
            }
        }
    }

    /// Stop a single systemd unit.
    async fn stop_unit(&self, unit: &str) -> Result<()> {
        let result = timeout(
            SYSTEMCTL_TIMEOUT,
            Command::new("systemctl")
                .args(["stop", unit])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) if output.status.success() => {
                debug!(unit, "Service stopped");
                Ok(())
            }
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(anyhow!("Failed to stop {}: {}", unit, stderr))
            }
            Ok(Err(e)) => Err(anyhow!("Failed to execute systemctl: {}", e)),
            Err(_) => Err(anyhow!("Timeout stopping {}", unit)),
        }
    }

    /// Wait for a service type to be ready (all units running).
    pub async fn wait_ready(&self, service_type: ServiceType, port: Option<u16>) -> Result<()> {
        let start = std::time::Instant::now();

        // First wait for systemd to report the service as active
        while start.elapsed() < SERVICE_READY_TIMEOUT {
            if self.is_running(service_type).await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        if !self.is_running(service_type).await {
            return Err(anyhow!("Service did not start within timeout"));
        }

        // If a port is specified, wait for it to be listening
        if let Some(port) = port {
            while start.elapsed() < SERVICE_READY_TIMEOUT {
                if self.is_port_listening(port).await {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            warn!(port, "Service started but port not listening, proceeding anyway");
        }

        Ok(())
    }

    /// Check if a port is listening on localhost.
    async fn is_port_listening(&self, port: u16) -> bool {
        use tokio::net::TcpStream;
        let addr = format!("127.0.0.1:{}", port);
        TcpStream::connect(&addr).await.is_ok()
    }
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self {
            app_units: Vec::new(),
            db_units: Vec::new(),
        }
    }
}
