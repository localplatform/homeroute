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
pub struct PowersaveManager {
    /// Service manager for starting/stopping services.
    service_mgr: Arc<RwLock<ServiceManager>>,

    /// Last activity time for code-server.
    last_code_server_activity: RwLock<Instant>,
    /// Last activity time for app/db.
    last_app_activity: RwLock<Instant>,

    /// Idle timeout for code-server (None = disabled).
    code_server_timeout: RwLock<Option<Duration>>,
    /// Idle timeout for app/db (None = disabled).
    app_timeout: RwLock<Option<Duration>>,

    /// code-server is manually stopped (no auto-wake).
    code_server_manually_off: AtomicBool,
    /// App is manually stopped (no auto-wake).
    app_manually_off: AtomicBool,
    /// DB is manually stopped (no auto-wake).
    db_manually_off: AtomicBool,

    /// Current state of code-server.
    code_server_state: RwLock<ServiceState>,
    /// Current state of app services.
    app_state: RwLock<ServiceState>,
    /// Current state of db services.
    db_state: RwLock<ServiceState>,
}

impl PowersaveManager {
    /// Create a new powersave manager.
    pub fn new(service_mgr: Arc<RwLock<ServiceManager>>) -> Self {
        let now = Instant::now();
        Self {
            service_mgr,
            last_code_server_activity: RwLock::new(now),
            last_app_activity: RwLock::new(now),
            code_server_timeout: RwLock::new(None),
            app_timeout: RwLock::new(None),
            code_server_manually_off: AtomicBool::new(false),
            app_manually_off: AtomicBool::new(false),
            db_manually_off: AtomicBool::new(false),
            code_server_state: RwLock::new(ServiceState::Stopped),
            app_state: RwLock::new(ServiceState::Stopped),
            db_state: RwLock::new(ServiceState::Stopped),
        }
    }

    /// Update power policy from registry.
    pub fn set_policy(&self, policy: &PowerPolicy) {
        *self.code_server_timeout.write().unwrap() = policy.code_server_idle_timeout_secs.map(Duration::from_secs);
        *self.app_timeout.write().unwrap() = policy.app_idle_timeout_secs.map(Duration::from_secs);
        info!(
            code_server_timeout = ?policy.code_server_idle_timeout_secs,
            app_timeout = ?policy.app_idle_timeout_secs,
            "Power policy updated"
        );
    }

    /// Record activity for a service type.
    pub fn record_activity(&self, service_type: ServiceType) {
        let now = Instant::now();
        match service_type {
            ServiceType::CodeServer => {
                *self.last_code_server_activity.write().unwrap() = now;
            }
            ServiceType::App | ServiceType::Db => {
                *self.last_app_activity.write().unwrap() = now;
            }
        }
    }

    /// Get idle seconds for a service type.
    pub fn idle_secs(&self, service_type: ServiceType) -> u64 {
        let last_activity = match service_type {
            ServiceType::CodeServer => *self.last_code_server_activity.read().unwrap(),
            ServiceType::App | ServiceType::Db => *self.last_app_activity.read().unwrap(),
        };
        last_activity.elapsed().as_secs()
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

    /// Check if a service is manually off.
    pub fn is_manually_off(&self, service_type: ServiceType) -> bool {
        match service_type {
            ServiceType::CodeServer => self.code_server_manually_off.load(Ordering::Relaxed),
            ServiceType::App => self.app_manually_off.load(Ordering::Relaxed),
            ServiceType::Db => self.db_manually_off.load(Ordering::Relaxed),
        }
    }

    /// Set manually off flag.
    fn set_manually_off(&self, service_type: ServiceType, value: bool) {
        match service_type {
            ServiceType::CodeServer => self.code_server_manually_off.store(value, Ordering::Relaxed),
            ServiceType::App => self.app_manually_off.store(value, Ordering::Relaxed),
            ServiceType::Db => self.db_manually_off.store(value, Ordering::Relaxed),
        }
    }

    /// Ensure a service is running for an incoming request.
    /// Returns WakeResult indicating what action was taken.
    /// Takes Arc<Self> to allow state updates from spawned tasks.
    /// `target_port` is the port to wait for before marking as running.
    pub async fn ensure_running(self: &Arc<Self>, service_type: ServiceType, target_port: u16) -> WakeResult {
        // Check if manually off
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

        // When starting App, ensure DB is started first (if configured and not manually off)
        if service_type == ServiceType::App
            && mgr.is_configured(ServiceType::Db)
            && !self.is_manually_off(ServiceType::Db)
        {
            let db_state = self.get_state(ServiceType::Db);
            if db_state != ServiceState::Running && db_state != ServiceState::Starting {
                info!("Auto-waking DB first (dependency of App)");
                self.set_state(ServiceType::Db, ServiceState::Starting);

                let mgr_clone = mgr.clone();
                let pm_clone = Arc::clone(self);
                tokio::spawn(async move {
                    if let Err(e) = mgr_clone.start(ServiceType::Db).await {
                        error!(error = %e, "Failed to auto-start DB");
                        pm_clone.set_state(ServiceType::Db, ServiceState::Stopped);
                    } else {
                        pm_clone.set_state(ServiceType::Db, ServiceState::Running);
                    }
                });
                self.record_activity(ServiceType::Db);
            }
        }

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

                // When starting App, ensure DB is started first (if configured)
                if service_type == ServiceType::App && mgr.is_configured(ServiceType::Db) {
                    let db_state = self.get_state(ServiceType::Db);
                    if db_state != ServiceState::Running {
                        info!("Starting DB first (dependency of App)");
                        self.set_manually_off(ServiceType::Db, false);
                        self.set_state(ServiceType::Db, ServiceState::Starting);

                        if let Err(e) = mgr.start(ServiceType::Db).await {
                            error!(error = %e, "Failed to start DB");
                            self.set_state(ServiceType::Db, ServiceState::Stopped);
                        } else {
                            self.set_state(ServiceType::Db, ServiceState::Running);
                            self.record_activity(ServiceType::Db);
                            // Wait a bit for DB to be ready
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        }

                        let _ = state_tx
                            .send(ServiceStateChange {
                                service_type: ServiceType::Db,
                                new_state: self.get_state(ServiceType::Db),
                            })
                            .await;
                    }
                }

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
                // Don't set manually_off - allow auto-wake on next request
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
    pub async fn run_idle_checker(self: Arc<Self>, state_tx: mpsc::Sender<ServiceStateChange>) {
        info!("Starting idle checker task");

        loop {
            tokio::time::sleep(IDLE_CHECK_INTERVAL).await;

            // Refresh actual service states from systemd
            self.refresh_states().await;

            // Read timeouts without holding the guard across await
            let code_server_timeout = *self.code_server_timeout.read().unwrap();
            let app_timeout = *self.app_timeout.read().unwrap();

            // Check code-server idle
            if let Some(timeout) = code_server_timeout {
                self.check_idle_and_stop(ServiceType::CodeServer, timeout, &state_tx).await;
            }

            // Check app/db idle (they share the same timeout)
            if let Some(timeout) = app_timeout {
                self.check_idle_and_stop(ServiceType::App, timeout, &state_tx).await;
                self.check_idle_and_stop(ServiceType::Db, timeout, &state_tx).await;
            }
        }
    }

    /// Refresh service states from systemd.
    async fn refresh_states(&self) {
        // Clone the manager to avoid holding the guard across await
        let mgr = self.service_mgr.read().unwrap().clone();

        // Only refresh if not manually off
        if !self.code_server_manually_off.load(Ordering::Relaxed) {
            let state = mgr.get_state(ServiceType::CodeServer).await;
            *self.code_server_state.write().unwrap() = state;
        }

        if !self.app_manually_off.load(Ordering::Relaxed) {
            let state = mgr.get_state(ServiceType::App).await;
            *self.app_state.write().unwrap() = state;
        }

        if !self.db_manually_off.load(Ordering::Relaxed) {
            let state = mgr.get_state(ServiceType::Db).await;
            *self.db_state.write().unwrap() = state;
        }
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
