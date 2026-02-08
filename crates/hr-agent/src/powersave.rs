//! Power-save management: idle tracking, auto-stop, and wake-on-request.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use hr_registry::protocol::{PowerPolicy, ServiceAction, ServiceState, ServiceType};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::services::ServiceManager;

/// Check interval for idle timeout.
const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Result of ensuring a service is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeResult {
    /// Service was already running.
    AlreadyRunning,
    /// Service was stopped, now starting.
    Starting,
    /// Service is manually stopped (no auto-wake).
    ManuallyOff,
}

/// Notification when a service state changes.
#[derive(Debug, Clone)]
pub struct ServiceStateChange {
    pub service_type: ServiceType,
    pub new_state: ServiceState,
}

/// Manages power-saving state and policies.
/// Only code-server has idle tracking and auto-stop. App/Db are managed
/// via direct service commands without idle timeout logic.
pub struct PowersaveManager {
    /// Service manager for starting/stopping services.
    service_mgr: Arc<RwLock<ServiceManager>>,

    /// Last activity time for code-server.
    last_code_server_activity: RwLock<Instant>,

    /// Idle timeout for code-server (None = disabled).
    code_server_timeout: RwLock<Option<Duration>>,

    /// code-server is manually stopped (no auto-wake).
    code_server_manually_off: AtomicBool,

    /// Current state of code-server.
    code_server_state: RwLock<ServiceState>,
    /// Current state of app services.
    app_state: RwLock<ServiceState>,
    /// Current state of db services.
    db_state: RwLock<ServiceState>,
}

/// Check if code-server has active non-loopback TCP connections on port 13337.
async fn has_active_code_server_connections() -> bool {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let output = tokio::process::Command::new("ss")
            .args(["-tn", "state", "established", "sport", "=", ":13337"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .await?;
        Ok::<_, std::io::Error>(output)
    })
    .await;

    let output = match result {
        Ok(Ok(output)) => output,
        _ => return false,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .skip(1)
        .any(|line| !line.contains("127.0.0.1") && !line.contains("::1"))
}

impl PowersaveManager {
    /// Create a new powersave manager.
    pub fn new(service_mgr: Arc<RwLock<ServiceManager>>) -> Self {
        let now = Instant::now();
        Self {
            service_mgr,
            last_code_server_activity: RwLock::new(now),
            code_server_timeout: RwLock::new(None),
            code_server_manually_off: AtomicBool::new(false),
            code_server_state: RwLock::new(ServiceState::Stopped),
            app_state: RwLock::new(ServiceState::Stopped),
            db_state: RwLock::new(ServiceState::Stopped),
        }
    }

    /// Update power policy from registry.
    pub fn set_policy(&self, policy: &PowerPolicy) {
        *self.code_server_timeout.write().unwrap() = policy.code_server_idle_timeout_secs.map(Duration::from_secs);
        info!(
            code_server_timeout = ?policy.code_server_idle_timeout_secs,
            "Power policy updated"
        );
    }

    /// Record activity for a service type (only meaningful for CodeServer).
    pub fn record_activity(&self, service_type: ServiceType) {
        if service_type == ServiceType::CodeServer {
            *self.last_code_server_activity.write().unwrap() = Instant::now();
        }
    }

    /// Get idle seconds for a service type.
    pub fn idle_secs(&self, service_type: ServiceType) -> u64 {
        match service_type {
            ServiceType::CodeServer => self.last_code_server_activity.read().unwrap().elapsed().as_secs(),
            ServiceType::App | ServiceType::Db => 0,
        }
    }

    /// Get the current state of a service type.
    pub fn get_state(&self, service_type: ServiceType) -> ServiceState {
        match service_type {
            ServiceType::CodeServer => *self.code_server_state.read().unwrap(),
            ServiceType::App => *self.app_state.read().unwrap(),
            ServiceType::Db => *self.db_state.read().unwrap(),
        }
    }

    /// Set the state of a service type.
    fn set_state(&self, service_type: ServiceType, state: ServiceState) {
        match service_type {
            ServiceType::CodeServer => *self.code_server_state.write().unwrap() = state,
            ServiceType::App => *self.app_state.write().unwrap() = state,
            ServiceType::Db => *self.db_state.write().unwrap() = state,
        }
    }

    /// Check if a service is manually off (only applies to CodeServer).
    pub fn is_manually_off(&self, service_type: ServiceType) -> bool {
        match service_type {
            ServiceType::CodeServer => self.code_server_manually_off.load(Ordering::Relaxed),
            ServiceType::App | ServiceType::Db => false,
        }
    }

    /// Set manually off flag (only applies to CodeServer).
    fn set_manually_off(&self, service_type: ServiceType, value: bool) {
        if service_type == ServiceType::CodeServer {
            self.code_server_manually_off.store(value, Ordering::Relaxed);
        }
    }

    /// Ensure a service is running for an incoming request.
    /// Returns WakeResult indicating what action was taken.
    /// Takes Arc<Self> to allow state updates from spawned tasks.
    /// `target_port` is the port to wait for before marking as running.
    pub async fn ensure_running(self: &Arc<Self>, service_type: ServiceType, target_port: u16) -> WakeResult {
        // Check if manually off (only applies to CodeServer)
        if self.is_manually_off(service_type) {
            return WakeResult::ManuallyOff;
        }

        // Check current state
        let current_state = self.get_state(service_type);
        match current_state {
            ServiceState::Running => {
                self.record_activity(service_type);
                return WakeResult::AlreadyRunning;
            }
            ServiceState::Starting => {
                return WakeResult::Starting;
            }
            ServiceState::Stopped | ServiceState::ManuallyOff | ServiceState::Stopping => {
                // Need to start
            }
        }

        // Clone the manager to avoid holding the lock across await
        let mgr = self.service_mgr.read().unwrap().clone();

        // Mark as starting
        self.set_state(service_type, ServiceState::Starting);
        info!(service_type = ?service_type, port = target_port, "Auto-waking service");

        // Clone Arc for the spawned task
        let pm_clone = Arc::clone(self);
        let st = service_type;
        let port = target_port;

        tokio::spawn(async move {
            // Start the service
            if let Err(e) = mgr.start(st).await {
                error!(service_type = ?st, error = %e, "Failed to start service");
                pm_clone.set_state(st, ServiceState::Stopped);
                return;
            }

            // Wait for the port to be ready
            info!(service_type = ?st, port = port, "Waiting for service port...");
            let start = std::time::Instant::now();
            let timeout = std::time::Duration::from_secs(60);

            while start.elapsed() < timeout {
                if tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await.is_ok() {
                    info!(service_type = ?st, port = port, elapsed_ms = start.elapsed().as_millis(), "Service port ready");
                    pm_clone.set_state(st, ServiceState::Running);
                    pm_clone.record_activity(st);
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }

            warn!(service_type = ?st, port = port, "Service started but port not listening after 60s");
            // Still mark as running after timeout, let the proxy return 502 if needed
            pm_clone.set_state(st, ServiceState::Running);
            pm_clone.record_activity(st);
        });

        self.record_activity(service_type);
        WakeResult::Starting
    }

    /// Handle a manual service command from the registry.
    pub async fn handle_command(
        &self,
        service_type: ServiceType,
        action: ServiceAction,
        state_tx: &mpsc::Sender<ServiceStateChange>,
    ) {
        // Clone the manager to avoid holding the guard across await
        let mgr = self.service_mgr.read().unwrap().clone();

        match action {
            ServiceAction::Start => {
                info!(service_type = ?service_type, "Manual start command");

                self.set_manually_off(service_type, false);
                self.set_state(service_type, ServiceState::Starting);

                if let Err(e) = mgr.start(service_type).await {
                    error!(service_type = ?service_type, error = %e, "Failed to start service");
                    self.set_state(service_type, ServiceState::Stopped);
                } else {
                    self.set_state(service_type, ServiceState::Running);
                    self.record_activity(service_type);
                }

                let _ = state_tx
                    .send(ServiceStateChange {
                        service_type,
                        new_state: self.get_state(service_type),
                    })
                    .await;
            }
            ServiceAction::Stop => {
                info!(service_type = ?service_type, "Manual stop command");
                self.set_state(service_type, ServiceState::Stopping);

                if let Err(e) = mgr.stop(service_type).await {
                    error!(service_type = ?service_type, error = %e, "Failed to stop service");
                }
                self.set_state(service_type, ServiceState::Stopped);

                let _ = state_tx
                    .send(ServiceStateChange {
                        service_type,
                        new_state: ServiceState::Stopped,
                    })
                    .await;
            }
        }
    }

    /// Background task that checks for idle services and stops them.
    /// Only code-server has idle timeout logic.
    pub async fn run_idle_checker(self: Arc<Self>, state_tx: mpsc::Sender<ServiceStateChange>) {
        info!("Starting idle checker task");

        loop {
            tokio::time::sleep(IDLE_CHECK_INTERVAL).await;

            // Refresh actual service states from systemd
            self.refresh_states().await;

            // Only check code-server idle timeout
            let code_server_timeout = *self.code_server_timeout.read().unwrap();
            if let Some(timeout) = code_server_timeout {
                self.check_idle_and_stop(ServiceType::CodeServer, timeout, &state_tx).await;
            }
        }
    }

    /// Refresh service states from systemd.
    async fn refresh_states(&self) {
        // Clone the manager to avoid holding the guard across await
        let mgr = self.service_mgr.read().unwrap().clone();

        // Only refresh code-server if not manually off
        if !self.code_server_manually_off.load(Ordering::Relaxed) {
            let state = mgr.get_state(ServiceType::CodeServer).await;
            *self.code_server_state.write().unwrap() = state;
        }

        // App/Db always refresh from systemd (no manually_off tracking)
        let state = mgr.get_state(ServiceType::App).await;
        *self.app_state.write().unwrap() = state;

        let state = mgr.get_state(ServiceType::Db).await;
        *self.db_state.write().unwrap() = state;
    }

    /// Check if a service is idle and stop it if so.
    async fn check_idle_and_stop(
        &self,
        service_type: ServiceType,
        timeout: Duration,
        state_tx: &mpsc::Sender<ServiceStateChange>,
    ) {
        // Don't stop manually off services (already stopped)
        if self.is_manually_off(service_type) {
            return;
        }

        // Only stop if currently running
        let state = self.get_state(service_type);
        if state != ServiceState::Running {
            return;
        }

        // Check idle time
        let idle = self.idle_secs(service_type);
        debug!(service_type = ?service_type, idle_secs = idle, timeout_secs = timeout.as_secs(), "Checking idle timeout");
        if idle < timeout.as_secs() {
            return;
        }

        // Check if service is configured
        let is_configured = {
            let mgr = self.service_mgr.read().unwrap();
            mgr.is_configured(service_type)
        };
        if !is_configured {
            return;
        }

        // Check for active WebSocket connections before stopping code-server
        if service_type == ServiceType::CodeServer && has_active_code_server_connections().await {
            debug!(service_type = ?service_type, idle_secs = idle, "Active connections detected, extending idle timer");
            self.record_activity(service_type);
            return;
        }

        info!(
            service_type = ?service_type,
            idle_secs = idle,
            timeout_secs = timeout.as_secs(),
            "Service idle, stopping for powersave"
        );

        self.set_state(service_type, ServiceState::Stopping);

        // Clone the manager to avoid holding the guard across await
        let mgr = self.service_mgr.read().unwrap().clone();
        if let Err(e) = mgr.stop(service_type).await {
            warn!(service_type = ?service_type, error = %e, "Failed to stop idle service");
            return;
        }

        self.set_state(service_type, ServiceState::Stopped);

        let _ = state_tx
            .send(ServiceStateChange {
                service_type,
                new_state: ServiceState::Stopped,
            })
            .await;
    }
}
