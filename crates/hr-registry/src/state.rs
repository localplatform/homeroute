//! Agent registry: manages application lifecycle, agent connections,
//! and pushes config to agents.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

use hr_acme::AcmeManager;
use hr_common::config::EnvConfig;
use hr_common::events::{AgentMetricsEvent, AgentStatusEvent, AgentUpdateEvent, AgentUpdateStatus, EventBus, HostPowerEvent, HostPowerState, PowerAction, WakeResult};
use crate::protocol::{AgentMetrics, ContainerInfo, HostMetrics, HostRegistryMessage, NetworkInterfaceInfo, PowerPolicy, RegistryMessage, ServiceAction, ServiceState, ServiceType};
use crate::types::{
    AgentNotifyResult, AgentSkipResult, AgentStatus, AgentUpdateStatusInfo,
    Application, CreateApplicationRequest, RegistryState, UpdateApplicationRequest,
    UpdateBatchResult, UpdateStatusResult,
};

/// Tracks all active WebSocket connections for a single app_id.
/// Multiple connections can coexist (e.g. main agent + MCP tool connections).
/// Routes are only removed when the last connection closes.
struct AppConnections {
    /// Primary tx for sending commands to the agent (from the main agent with IPv4).
    tx: mpsc::Sender<RegistryMessage>,
    connected_at: DateTime<Utc>,
    last_heartbeat: DateTime<Utc>,
    /// Number of active WebSocket connections for this app_id.
    active_count: usize,
}

/// Wrapper for outgoing messages to host-agents: text (JSON) or raw binary.
#[derive(Debug)]
pub enum OutgoingHostMessage {
    Text(HostRegistryMessage),
    Binary(Vec<u8>),
}

/// In-memory host-agent connection state.
pub struct HostConnection {
    pub tx: mpsc::Sender<OutgoingHostMessage>,
    pub host_name: String,
    pub connected_at: DateTime<Utc>,
    pub last_heartbeat: DateTime<Utc>,
    pub version: Option<String>,
    pub metrics: Option<HostMetrics>,
    pub containers: Vec<ContainerInfo>,
    pub interfaces: Vec<NetworkInterfaceInfo>,
}

pub enum MigrationResult {
    ImportComplete { container_name: String },
    ImportFailed { error: String },
    ExportFailed { error: String },
}

/// Tracks power state of a remote host for WOL deduplication and conflict detection.
pub struct HostPowerInfo {
    pub state: HostPowerState,
    pub since: DateTime<Utc>,
    pub last_wol_sent: Option<DateTime<Utc>>,
    pub mac_address: Option<String>,
}

fn service_state_str(s: ServiceState) -> String {
    match s {
        ServiceState::Running => "running".to_string(),
        ServiceState::Stopped => "stopped".to_string(),
        ServiceState::Starting => "starting".to_string(),
        ServiceState::Stopping => "stopping".to_string(),
        ServiceState::ManuallyOff => "manually_off".to_string(),
    }
}

fn service_type_str(t: ServiceType) -> &'static str {
    match t {
        ServiceType::CodeServer => "code_server",
        ServiceType::App => "app",
        ServiceType::Db => "db",
    }
}

pub struct AgentRegistry {
    state: Arc<RwLock<RegistryState>>,
    state_path: PathBuf,
    connections: Arc<RwLock<HashMap<String, AppConnections>>>,
    pub host_connections: Arc<RwLock<HashMap<String, HostConnection>>>,
    env: Arc<EnvConfig>,
    events: Arc<EventBus>,
    migration_signals: Arc<RwLock<HashMap<String, tokio::sync::oneshot::Sender<MigrationResult>>>>,
    exec_signals: Arc<RwLock<HashMap<String, tokio::sync::oneshot::Sender<(bool, String, String)>>>>,
    /// Maps transfer_id → container_name for in-flight migrations (set when StartExport is sent)
    pub transfer_container_names: Arc<RwLock<HashMap<String, String>>>,
    /// Maps transfer_id → (target_host_id, container_name) for remote→remote relay migrations
    pub transfer_relay_targets: Arc<RwLock<HashMap<String, (String, String)>>>,
    /// Host power state machine for WOL dedup, conflict detection, and progress tracking.
    host_power_states: Arc<RwLock<HashMap<String, HostPowerInfo>>>,
    /// ACME manager for per-app wildcard certificate lifecycle.
    acme: RwLock<Option<Arc<AcmeManager>>>,
    /// Terminal sessions: maps session_id → sender for data from host-agent to API WS handler.
    terminal_sessions: Arc<RwLock<HashMap<String, mpsc::Sender<Vec<u8>>>>>,
    /// Dataverse query signals: maps request_id → oneshot sender for query results.
    dataverse_query_signals: Arc<RwLock<HashMap<String, tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>>>>,
}

impl AgentRegistry {
    /// Load or create the registry state from disk.
    pub fn new(
        state_path: PathBuf,
        env: Arc<EnvConfig>,
        events: Arc<EventBus>,
    ) -> Self {
        let state = match std::fs::read_to_string(&state_path) {
            Ok(content) => {
                serde_json::from_str(&content).unwrap_or_else(|e| {
                    warn!("Failed to parse registry state, starting fresh: {e}");
                    RegistryState::default()
                })
            }
            Err(_) => RegistryState::default(),
        };

        info!(
            apps = state.applications.len(),
            "Loaded agent registry state"
        );

        Self {
            state: Arc::new(RwLock::new(state)),
            state_path,
            connections: Arc::new(RwLock::new(HashMap::new())),
            host_connections: Arc::new(RwLock::new(HashMap::new())),
            env,
            events,
            migration_signals: Arc::new(RwLock::new(HashMap::new())),
            exec_signals: Arc::new(RwLock::new(HashMap::new())),
            transfer_container_names: Arc::new(RwLock::new(HashMap::new())),
            transfer_relay_targets: Arc::new(RwLock::new(HashMap::new())),
            host_power_states: Arc::new(RwLock::new(HashMap::new())),
            acme: RwLock::new(None),
            terminal_sessions: Arc::new(RwLock::new(HashMap::new())),
            dataverse_query_signals: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set the ACME manager for per-app wildcard certificate lifecycle.
    /// Called after the ACME manager is initialized in the main supervisor.
    pub async fn set_acme(&self, acme: Arc<AcmeManager>) {
        *self.acme.write().await = Some(acme);
        info!("ACME manager registered with agent registry");
    }

    /// Request a per-app wildcard certificate (*.{slug}.{base_domain}).
    /// Spawns a background task; non-blocking.
    pub async fn request_app_cert(&self, slug: &str) {
        let acme_guard = self.acme.read().await;
        if let Some(acme) = acme_guard.clone() {
            let slug_owned = slug.to_string();
            tokio::spawn(async move {
                match acme.request_app_wildcard(&slug_owned).await {
                    Ok(_cert) => {
                        info!(slug = %slug_owned, "Per-app wildcard certificate issued");
                    }
                    Err(e) => {
                        warn!(slug = %slug_owned, error = %e, "Failed to issue per-app wildcard certificate");
                    }
                }
            });
        }
    }

    // ── Application CRUD ────────────────────────────────────────

    /// Create an application record without spawning background deployment.
    /// Used by Containers V2 which manage their own deployment lifecycle.
    /// Returns the application (status=Deploying) and the cleartext token.
    pub async fn create_application_headless(
        self: &Arc<Self>,
        req: CreateApplicationRequest,
    ) -> Result<(Application, String)> {
        let token_clear = generate_token();
        let token_hash = hash_token(&token_clear)?;

        let id = uuid::Uuid::new_v4().to_string();
        let container_name = format!("hr-v2-{}", req.slug);

        let app = Application {
            id: id.clone(),
            name: req.name,
            slug: req.slug,
            host_id: req.host_id.unwrap_or_else(|| "local".to_string()),
            enabled: true,
            container_name: container_name.clone(),
            token_hash,
            ipv4_address: None,
            status: AgentStatus::Deploying,
            last_heartbeat: None,
            agent_version: None,
            created_at: Utc::now(),
            frontend: req.frontend,
            apis: req.apis,
            code_server_enabled: req.code_server_enabled,
            services: req.services,
            power_policy: req.power_policy,
            wake_page_enabled: req.wake_page_enabled,
            metrics: None,
        };

        {
            let mut state = self.state.write().await;
            state.applications.push(app.clone());
        }
        self.persist().await?;

        info!(app = app.slug, container = container_name, "Application created (headless)");

        Ok((app, token_clear))
    }

    /// Set an application's status and persist.
    async fn set_app_status(&self, app_id: &str, status: AgentStatus) {
        {
            let mut state = self.state.write().await;
            if let Some(app) = state.applications.iter_mut().find(|a| a.id == app_id) {
                app.status = status;
            }
        }
        let _ = self.persist().await;
    }

    /// Remove a failed application from state (cleanup after deploy failure).
    async fn remove_failed_app(&self, app_id: &str) {
        {
            let mut state = self.state.write().await;
            state.applications.retain(|a| a.id != app_id);
        }
        let _ = self.persist().await;
    }

    /// Update application endpoints/auth. Pushes new config to connected agent.
    pub async fn update_application(&self, id: &str, req: UpdateApplicationRequest) -> Result<Option<Application>> {
        let mut state = self.state.write().await;
        let Some(app) = state.applications.iter_mut().find(|a| a.id == id) else {
            return Ok(None);
        };

        if let Some(name) = req.name {
            app.name = name;
        }
        if let Some(host_id) = req.host_id {
            app.host_id = host_id;
        }
        if let Some(frontend) = req.frontend {
            app.frontend = frontend;
        }
        if let Some(apis) = req.apis {
            app.apis = apis;
        }
        if let Some(code_server_enabled) = req.code_server_enabled {
            app.code_server_enabled = code_server_enabled;
        }
        if let Some(services) = req.services {
            app.services = services;
        }
        if let Some(power_policy) = req.power_policy {
            app.power_policy = power_policy;
        }
        if let Some(wake_page_enabled) = req.wake_page_enabled {
            app.wake_page_enabled = wake_page_enabled;
        }

        let app = app.clone();
        drop(state);

        self.persist().await?;

        // Push new config to connected agent if any
        self.push_config_to_agent(&app).await;

        Ok(Some(app))
    }

    /// Remove an application: disconnect agent and clean up.
    pub async fn remove_application(&self, id: &str) -> Result<bool> {
        let app = {
            let mut state = self.state.write().await;
            let idx = state.applications.iter().position(|a| a.id == id);
            match idx {
                Some(i) => state.applications.remove(i),
                None => return Ok(false),
            }
        };

        // Send shutdown to agent if connected
        {
            let conns = self.connections.read().await;
            if let Some(conn) = conns.get(&app.id) {
                let _ = conn.tx.send(RegistryMessage::Shutdown).await;
            }
        }

        // Delete per-app wildcard certificate
        {
            let acme_guard = self.acme.read().await;
            if let Some(ref acme) = *acme_guard {
                if let Err(e) = acme.delete_app_certificate(&app.slug) {
                    warn!(slug = app.slug, error = %e, "Failed to delete app certificate");
                }
            }
        }

        // Delete per-app wildcard DNS record if Cloudflare credentials available
        if let (Some(token), Some(zone_id)) = (&self.env.cf_api_token, &self.env.cf_zone_id) {
            if let Err(e) = crate::cloudflare::delete_app_wildcard_dns(
                token,
                zone_id,
                &app.slug,
                &self.env.base_domain,
            ).await {
                warn!(slug = app.slug, error = %e, "Failed to delete app wildcard DNS");
            }
        }

        self.persist().await?;
        info!(app = app.slug, "Application removed");
        Ok(true)
    }

    pub async fn list_applications(&self) -> Vec<Application> {
        self.state.read().await.applications.clone()
    }

    pub async fn toggle_application(&self, id: &str) -> Result<Option<bool>> {
        let mut state = self.state.write().await;
        let Some(app) = state.applications.iter_mut().find(|a| a.id == id) else {
            return Ok(None);
        };
        app.enabled = !app.enabled;
        let enabled = app.enabled;
        drop(state);
        self.persist().await?;
        Ok(Some(enabled))
    }

    /// Regenerate the token for an application. Returns the new cleartext token.
    pub async fn regenerate_token(&self, id: &str) -> Result<Option<String>> {
        let token_clear = generate_token();
        let token_hash = hash_token(&token_clear)?;

        let mut state = self.state.write().await;
        let Some(app) = state.applications.iter_mut().find(|a| a.id == id) else {
            return Ok(None);
        };
        app.token_hash = token_hash;
        drop(state);

        self.persist().await?;
        info!(app_id = id, "Token regenerated");
        Ok(Some(token_clear))
    }

    // ── Agent connection lifecycle ──────────────────────────────

    /// Authenticate an agent by token and service name.
    pub async fn authenticate(&self, token: &str, service_name: &str) -> Option<String> {
        let state = self.state.read().await;
        for app in &state.applications {
            if app.slug == service_name && verify_token(token, &app.token_hash) {
                return Some(app.id.clone());
            }
        }
        None
    }

    /// Called when an agent successfully connects and authenticates.
    /// Pushes simplified config (services + power_policy).
    pub async fn on_agent_connected(
        &self,
        app_id: &str,
        tx: mpsc::Sender<RegistryMessage>,
        agent_version: String,
        reported_ipv4: Option<String>,
    ) -> Result<()> {
        let now = Utc::now();

        // Increment connection count (or create new entry).
        // Only overwrite the primary tx if this connection has an IPv4 (real agent).
        {
            let mut conns = self.connections.write().await;
            if let Some(existing) = conns.get_mut(app_id) {
                existing.active_count += 1;
                existing.last_heartbeat = now;
                // Only overwrite tx if this is the main agent (has IPv4)
                if reported_ipv4.is_some() {
                    existing.tx = tx.clone();
                }
                info!(app_id, count = existing.active_count, has_ipv4 = reported_ipv4.is_some(),
                    "Additional agent connection registered");
            } else {
                conns.insert(
                    app_id.to_string(),
                    AppConnections {
                        tx: tx.clone(),
                        connected_at: now,
                        last_heartbeat: now,
                        active_count: 1,
                    },
                );
            }
        }

        // Update status and IPv4 address
        let app = {
            let mut state = self.state.write().await;
            let app = state.applications.iter_mut().find(|a| a.id == app_id);
            if let Some(app) = app {
                app.status = AgentStatus::Connected;
                app.agent_version = Some(agent_version);
                app.last_heartbeat = Some(now);

                if let Some(ref ipv4_str) = reported_ipv4 {
                    if let Ok(addr) = ipv4_str.parse() {
                        app.ipv4_address = Some(addr);
                        info!(app_id, ipv4 = ipv4_str, "Updated app IPv4 from agent report");
                    }
                }

                Some(app.clone())
            } else {
                None
            }
        };

        // Notify frontend via WebSocket
        if let Some(ref app) = app {
            let _ = self.events.agent_status.send(AgentStatusEvent {
                app_id: app_id.to_string(),
                slug: app.slug.clone(),
                status: "connected".to_string(),
                message: None,
            });
        }

        // Push config with endpoint info for agent route publishing
        if let Some(app) = app {
            let _ = tx
                .send(RegistryMessage::Config {
                    config_version: 1,
                    services: app.services.clone(),
                    power_policy: app.power_policy.clone(),
                    base_domain: self.env.base_domain.clone(),
                    slug: app.slug.clone(),
                    frontend: Some(app.frontend.clone()),
                    apis: app.apis.clone(),
                    code_server_enabled: app.code_server_enabled,
                    wake_page_enabled: app.wake_page_enabled,
                })
                .await;
        }

        if let Err(e) = self.persist().await {
            warn!(app_id, "Failed to persist registry state on connect: {e}");
        }
        Ok(())
    }

    /// Called when an agent WebSocket disconnects.
    /// Decrements the active connection count. Only performs full cleanup
    /// (status=Disconnected, route removal) when the last connection closes.
    /// Returns true if this was the last connection and routes should be removed.
    pub async fn on_agent_disconnected(&self, app_id: &str) -> bool {
        let is_last = {
            let mut conns = self.connections.write().await;
            if let Some(existing) = conns.get_mut(app_id) {
                existing.active_count = existing.active_count.saturating_sub(1);
                if existing.active_count == 0 {
                    conns.remove(app_id);
                    true
                } else {
                    info!(app_id, remaining = existing.active_count,
                        "Agent connection closed, others still active (routes preserved)");
                    false
                }
            } else {
                // No entry — nothing to clean up
                return false;
            }
        };

        if !is_last {
            return false;
        }

        let slug = {
            let mut state = self.state.write().await;
            if let Some(app) = state.applications.iter_mut().find(|a| a.id == app_id) {
                app.status = AgentStatus::Disconnected;
                Some(app.slug.clone())
            } else {
                None
            }
        };

        // Notify frontend via WebSocket
        if let Some(slug) = slug {
            let _ = self.events.agent_status.send(AgentStatusEvent {
                app_id: app_id.to_string(),
                slug,
                status: "disconnected".to_string(),
                message: None,
            });
        }

        let _ = self.persist().await;
        info!(app_id, "Agent disconnected (last connection, routes will be removed)");
        true
    }

    /// Update heartbeat timestamp for an agent.
    /// If the agent was marked stale but is still sending heartbeats
    /// (e.g. after a host suspend/resume), restore its Connected status.
    pub async fn handle_heartbeat(&self, app_id: &str) {
        let now = Utc::now();
        {
            let mut conns = self.connections.write().await;
            if let Some(conn) = conns.get_mut(app_id) {
                conn.last_heartbeat = now;
            }
        }
        let reconnected_slug = {
            let mut state = self.state.write().await;
            if let Some(app) = state.applications.iter_mut().find(|a| a.id == app_id) {
                app.last_heartbeat = Some(now);
                if app.status == AgentStatus::Disconnected {
                    app.status = AgentStatus::Connected;
                    Some(app.slug.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(slug) = reconnected_slug {
            let _ = self.events.agent_status.send(AgentStatusEvent {
                app_id: app_id.to_string(),
                slug,
                status: "connected".to_string(),
                message: None,
            });
            let _ = self.persist().await;
            info!(app_id, "Agent reconnected after stale heartbeat");
        }
    }

    /// Mark an agent as stale (heartbeat timeout) without removing its connection.
    /// This allows the agent to auto-recover if the WebSocket is still alive
    /// (e.g. after a host suspend/resume cycle).
    async fn mark_agent_stale(&self, app_id: &str) {
        let slug = {
            let mut state = self.state.write().await;
            if let Some(app) = state.applications.iter_mut().find(|a| a.id == app_id) {
                if app.status == AgentStatus::Disconnected {
                    return; // Already marked stale
                }
                app.status = AgentStatus::Disconnected;
                Some(app.slug.clone())
            } else {
                None
            }
        };

        if let Some(slug) = slug {
            let _ = self.events.agent_status.send(AgentStatusEvent {
                app_id: app_id.to_string(),
                slug,
                status: "disconnected".to_string(),
                message: None,
            });
        }

        let _ = self.persist().await;
        warn!(app_id, "Agent marked stale (heartbeat timeout)");
    }

    /// Background task: check heartbeats and mark stale agents as disconnected.
    /// Also checks host power state timeouts.
    pub async fn run_heartbeat_monitor(self: &Arc<Self>) {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;

            let now = Utc::now();
            let stale_threshold = chrono::Duration::seconds(90);
            let mut stale_ids = Vec::new();

            {
                let conns = self.connections.read().await;
                for (id, conn) in conns.iter() {
                    if now - conn.last_heartbeat > stale_threshold {
                        stale_ids.push(id.clone());
                    }
                }
            }

            for id in stale_ids {
                self.mark_agent_stale(&id).await;
            }

            // Check host power state timeouts (WakingUp, Rebooting, etc.)
            self.check_power_state_timeouts().await;
        }
    }

    // ── Host-agent connection lifecycle ────────────────────────

    pub async fn on_host_connected(
        &self,
        host_id: String,
        host_name: String,
        tx: mpsc::Sender<OutgoingHostMessage>,
        version: String,
    ) {
        let conn = HostConnection {
            tx,
            host_name: host_name.clone(),
            connected_at: Utc::now(),
            last_heartbeat: Utc::now(),
            version: Some(version.clone()),
            metrics: None,
            containers: Vec::new(),
            interfaces: Vec::new(),
        };
        self.host_connections.write().await.insert(host_id.clone(), conn);

        // Update power state machine → Online
        self.transition_power_state(&host_id, HostPowerState::Online, "Hote connecte").await;

        info!("Host agent connected: {} ({})", host_name, host_id);
    }

    pub async fn on_host_disconnected(&self, host_id: &str) {
        // Transition based on current power state
        let new_state = {
            let states = self.host_power_states.read().await;
            match states.get(host_id).map(|s| s.state) {
                Some(HostPowerState::ShuttingDown) => HostPowerState::Offline,
                Some(HostPowerState::Rebooting) => HostPowerState::Rebooting, // stay, expect reconnection
                Some(HostPowerState::Suspending) => HostPowerState::Suspended,
                _ => HostPowerState::Offline,
            }
        };
        let msg = match new_state {
            HostPowerState::Rebooting => "Hote en redemarrage, en attente de reconnexion",
            HostPowerState::Suspended => "Hote en veille",
            _ => "Hote deconnecte",
        };
        self.transition_power_state(host_id, new_state, msg).await;

        if let Some(conn) = self.host_connections.write().await.remove(host_id) {
            info!("Host agent disconnected: {} ({})", conn.host_name, host_id);
        }
    }

    pub async fn is_host_connected(&self, host_id: &str) -> bool {
        self.host_connections.read().await.contains_key(host_id)
    }

    /// Check if an agent has an active WebSocket connection.
    pub async fn is_agent_connected(&self, app_id: &str) -> bool {
        self.connections.read().await.contains_key(app_id)
    }

    // ── Host power state machine ────────────────────────────────

    /// Get the current power state of a host.
    pub async fn get_host_power_state(&self, host_id: &str) -> HostPowerState {
        self.host_power_states
            .read()
            .await
            .get(host_id)
            .map(|s| s.state)
            .unwrap_or(HostPowerState::Offline)
    }

    /// Internal: transition power state and emit event.
    async fn transition_power_state(&self, host_id: &str, new_state: HostPowerState, message: &str) {
        let mut states = self.host_power_states.write().await;
        let entry = states.entry(host_id.to_string()).or_insert_with(|| HostPowerInfo {
            state: HostPowerState::Offline,
            since: Utc::now(),
            last_wol_sent: None,
            mac_address: None,
        });

        let old_state = entry.state;
        if old_state == new_state {
            return;
        }

        entry.state = new_state;
        entry.since = Utc::now();

        // Clear WOL tracking when going online or offline
        if matches!(new_state, HostPowerState::Online | HostPowerState::Offline) {
            entry.last_wol_sent = None;
        }

        info!(
            host_id,
            from = %old_state,
            to = %new_state,
            "Host power state transition"
        );

        let _ = self.events.host_power.send(HostPowerEvent {
            host_id: host_id.to_string(),
            state: new_state,
            message: message.to_string(),
        });
    }

    /// Request a host wake-up via WOL. Handles deduplication and conflict detection.
    pub async fn request_wake_host(&self, host_id: &str) -> Result<WakeResult, String> {
        let (current_state, last_wol, cached_mac) = {
            let states = self.host_power_states.read().await;
            match states.get(host_id) {
                Some(info) => (info.state, info.last_wol_sent, info.mac_address.clone()),
                None => (HostPowerState::Offline, None, None),
            }
        };

        match current_state {
            HostPowerState::Online => return Ok(WakeResult::AlreadyOnline),
            HostPowerState::ShuttingDown => return Err("L'hote est en cours d'arret".to_string()),
            HostPowerState::Rebooting => return Err("L'hote est en cours de redemarrage".to_string()),
            HostPowerState::Suspending => return Err("L'hote est en cours de mise en veille".to_string()),
            HostPowerState::WakingUp => {
                // Dedup: if WOL sent less than 30s ago, skip
                if let Some(sent_at) = last_wol {
                    if (Utc::now() - sent_at).num_seconds() < 30 {
                        return Ok(WakeResult::AlreadyWaking);
                    }
                }
                // Retry WOL after 30s
            }
            HostPowerState::Offline | HostPowerState::Suspended => {
                // Proceed to send WOL
            }
        }

        // Look up MAC address (use cache if available)
        let mac = match cached_mac {
            Some(m) => m,
            None => {
                let m = Self::lookup_host_mac(host_id).await
                    .ok_or_else(|| "Adresse MAC non trouvee".to_string())?;
                // Cache it
                let mut states = self.host_power_states.write().await;
                if let Some(info) = states.get_mut(host_id) {
                    info.mac_address = Some(m.clone());
                }
                m
            }
        };

        // Send WOL packet
        Self::send_wol_packet(&mac).await?;

        // Update state
        {
            let mut states = self.host_power_states.write().await;
            let entry = states.entry(host_id.to_string()).or_insert_with(|| HostPowerInfo {
                state: HostPowerState::Offline,
                since: Utc::now(),
                last_wol_sent: None,
                mac_address: Some(mac),
            });
            entry.last_wol_sent = Some(Utc::now());

            if entry.state != HostPowerState::WakingUp {
                entry.state = HostPowerState::WakingUp;
                entry.since = Utc::now();
            }
        }

        // Emit event
        let _ = self.events.host_power.send(HostPowerEvent {
            host_id: host_id.to_string(),
            state: HostPowerState::WakingUp,
            message: "Magic packet WOL envoye".to_string(),
        });

        info!(host_id, "WOL packet sent");
        Ok(WakeResult::WolSent)
    }

    /// Request a power action (shutdown/reboot/suspend). Validates state conflicts.
    pub async fn request_power_action(&self, host_id: &str, action: PowerAction) -> Result<(), String> {
        let current_state = self.get_host_power_state(host_id).await;

        // Check for conflicts
        match (current_state, action) {
            (HostPowerState::WakingUp, _) => {
                return Err("L'hote est en cours de reveil".to_string());
            }
            (HostPowerState::ShuttingDown, PowerAction::Shutdown) => {
                return Err("L'hote est deja en cours d'arret".to_string());
            }
            (HostPowerState::Rebooting, PowerAction::Reboot) => {
                return Err("L'hote est deja en cours de redemarrage".to_string());
            }
            (HostPowerState::Suspending, PowerAction::Suspend) => {
                return Err("L'hote est deja en cours de mise en veille".to_string());
            }
            (HostPowerState::Offline | HostPowerState::Suspended, _) => {
                return Err("L'hote est hors ligne".to_string());
            }
            _ => {}
        }

        let (new_state, message) = match action {
            PowerAction::Shutdown => (HostPowerState::ShuttingDown, "Arret en cours"),
            PowerAction::Reboot => (HostPowerState::Rebooting, "Redemarrage en cours"),
            PowerAction::Suspend => (HostPowerState::Suspending, "Mise en veille en cours"),
        };

        self.transition_power_state(host_id, new_state, message).await;
        Ok(())
    }

    /// Invalidate the cached MAC address for a host (called when user changes WOL MAC).
    pub async fn invalidate_host_mac_cache(&self, host_id: &str) {
        let mut states = self.host_power_states.write().await;
        if let Some(info) = states.get_mut(host_id) {
            info.mac_address = None;
        }
    }

    /// Check power state timeouts (called from heartbeat monitor loop).
    async fn check_power_state_timeouts(&self) {
        let now = Utc::now();
        let mut transitions = Vec::new();

        {
            let states = self.host_power_states.read().await;
            for (host_id, info) in states.iter() {
                let elapsed = (now - info.since).num_seconds();
                let timeout = match info.state {
                    HostPowerState::WakingUp => 180,
                    HostPowerState::Rebooting => 120,
                    HostPowerState::ShuttingDown => 60,
                    HostPowerState::Suspending => 30,
                    _ => continue,
                };
                if elapsed > timeout {
                    transitions.push((host_id.clone(), info.state));
                }
            }
        }

        for (host_id, timed_out_state) in transitions {
            let msg = match timed_out_state {
                HostPowerState::WakingUp => "Timeout: l'hote n'a pas repondu au WOL",
                HostPowerState::Rebooting => "Timeout: l'hote n'a pas redemarré",
                HostPowerState::ShuttingDown => "Timeout: arret presume termine",
                HostPowerState::Suspending => "Timeout: mise en veille presumee terminee",
                _ => continue,
            };
            warn!(host_id = %host_id, state = %timed_out_state, "Power state timeout");
            self.transition_power_state(&host_id, HostPowerState::Offline, msg).await;
        }
    }

    /// Send a Wake-on-LAN magic packet to the given MAC address.
    pub async fn send_wol_packet(mac: &str) -> Result<(), String> {
        let mac_bytes: Vec<u8> = mac
            .split(':')
            .filter_map(|b| u8::from_str_radix(b, 16).ok())
            .collect();

        if mac_bytes.len() != 6 {
            return Err("Adresse MAC invalide".to_string());
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

    /// Look up the MAC address for a host from /data/hosts.json.
    pub async fn lookup_host_mac(host_id: &str) -> Option<String> {
        let content = tokio::fs::read_to_string("/data/hosts.json").await.ok()?;
        let data: serde_json::Value = serde_json::from_str(&content).ok()?;
        let hosts = data.get("hosts")?.as_array()?;
        let host = hosts.iter().find(|h| {
            h.get("id").and_then(|i| i.as_str()) == Some(host_id)
        })?;
        // Prefer wol_mac, fall back to mac
        host.get("wol_mac")
            .and_then(|m| m.as_str())
            .or_else(|| host.get("mac").and_then(|m| m.as_str()))
            .map(|s| s.to_string())
    }

    pub async fn update_host_heartbeat(&self, host_id: &str) {
        if let Some(conn) = self.host_connections.write().await.get_mut(host_id) {
            conn.last_heartbeat = Utc::now();
        }
    }

    pub async fn update_host_metrics(&self, host_id: &str, metrics: HostMetrics) {
        if let Some(conn) = self.host_connections.write().await.get_mut(host_id) {
            conn.metrics = Some(metrics);
        }
    }

    pub async fn update_host_containers(&self, host_id: &str, containers: Vec<ContainerInfo>) {
        if let Some(conn) = self.host_connections.write().await.get_mut(host_id) {
            conn.containers = containers;
        }
    }

    pub async fn update_host_interfaces(&self, host_id: &str, interfaces: Vec<NetworkInterfaceInfo>) {
        if let Some(conn) = self.host_connections.write().await.get_mut(host_id) {
            conn.interfaces = interfaces;
        }
    }

    pub async fn send_host_command(
        &self,
        host_id: &str,
        msg: HostRegistryMessage,
    ) -> Result<(), String> {
        // Clone the sender and release the lock BEFORE sending,
        // to avoid holding host_connections read lock during channel send
        // (which would deadlock with heartbeat/status write locks).
        let tx = {
            let conns = self.host_connections.read().await;
            match conns.get(host_id) {
                Some(conn) => conn.tx.clone(),
                None => return Err(format!("Host {} not connected", host_id)),
            }
        };
        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            tx.send(OutgoingHostMessage::Text(msg)),
        ).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(format!("Failed to send to host {}: {}", host_id, e)),
            Err(_) => Err(format!("Timeout sending to host {} (channel full for 30s)", host_id)),
        }
    }

    /// Send raw binary data to a host-agent (for migration chunk relay).
    pub async fn send_host_binary(
        &self,
        host_id: &str,
        data: Vec<u8>,
    ) -> Result<(), String> {
        let tx = {
            let conns = self.host_connections.read().await;
            match conns.get(host_id) {
                Some(conn) => conn.tx.clone(),
                None => return Err(format!("Host {} not connected", host_id)),
            }
        };
        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            tx.send(OutgoingHostMessage::Binary(data)),
        ).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(format!("Failed to send binary to host {}: {}", host_id, e)),
            Err(_) => Err(format!("Timeout sending binary to host {} (channel full for 30s)", host_id)),
        }
    }

    // ── Migration & exec signal handling ──────────────────────

    pub async fn register_migration_signal(&self, transfer_id: &str) -> tokio::sync::oneshot::Receiver<MigrationResult> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.migration_signals.write().await.insert(transfer_id.to_string(), tx);
        rx
    }

    /// Store the container_name for a given transfer_id (called when StartExport is sent).
    pub async fn set_transfer_container_name(&self, transfer_id: &str, container_name: &str) {
        self.transfer_container_names.write().await.insert(transfer_id.to_string(), container_name.to_string());
    }

    /// Retrieve and remove the container_name for a given transfer_id.
    pub async fn take_transfer_container_name(&self, transfer_id: &str) -> Option<String> {
        self.transfer_container_names.write().await.remove(transfer_id)
    }

    /// Store relay target for remote→remote migration (transfer_id → (target_host_id, container_name)).
    pub async fn set_transfer_relay_target(&self, transfer_id: &str, target_host_id: &str, container_name: &str) {
        self.transfer_relay_targets.write().await.insert(
            transfer_id.to_string(),
            (target_host_id.to_string(), container_name.to_string()),
        );
    }

    /// Get relay target for a transfer (non-destructive read).
    pub async fn get_transfer_relay_target(&self, transfer_id: &str) -> Option<(String, String)> {
        self.transfer_relay_targets.read().await.get(transfer_id).cloned()
    }

    /// Remove relay target for a completed/failed transfer.
    pub async fn take_transfer_relay_target(&self, transfer_id: &str) -> Option<(String, String)> {
        self.transfer_relay_targets.write().await.remove(transfer_id)
    }

    pub async fn on_host_import_complete(&self, _host_id: &str, transfer_id: &str, container_name: &str) {
        if let Some(tx) = self.migration_signals.write().await.remove(transfer_id) {
            let _ = tx.send(MigrationResult::ImportComplete { container_name: container_name.to_string() });
        }
    }

    pub async fn on_host_import_failed(&self, _host_id: &str, transfer_id: &str, error: &str) {
        if let Some(tx) = self.migration_signals.write().await.remove(transfer_id) {
            let _ = tx.send(MigrationResult::ImportFailed { error: error.to_string() });
        }
    }

    pub async fn on_host_export_failed(&self, _host_id: &str, transfer_id: &str, error: &str) {
        if let Some(tx) = self.migration_signals.write().await.remove(transfer_id) {
            let _ = tx.send(MigrationResult::ExportFailed { error: error.to_string() });
        }
    }

    pub async fn on_host_exec_result(&self, _host_id: &str, request_id: &str, success: bool, stdout: &str, stderr: &str) {
        if let Some(tx) = self.exec_signals.write().await.remove(request_id) {
            let _ = tx.send((success, stdout.to_string(), stderr.to_string()));
        }
    }

    pub async fn exec_in_remote_container(&self, host_id: &str, container_name: &str, command: Vec<String>) -> Result<(bool, String, String)> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.exec_signals.write().await.insert(request_id.clone(), tx);

        self.send_host_command(host_id, crate::protocol::HostRegistryMessage::ExecInContainer {
            request_id: request_id.clone(),
            container_name: container_name.to_string(),
            command,
        }).await.map_err(|e| anyhow::anyhow!("{}", e))?;

        match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => {
                anyhow::bail!("Exec signal channel closed");
            }
            Err(_) => {
                self.exec_signals.write().await.remove(&request_id);
                anyhow::bail!("Exec timeout after 60s");
            }
        }
    }

    /// Look up an application by id.
    pub async fn get_application(&self, id: &str) -> Option<Application> {
        let state = self.state.read().await;
        state.applications.iter().find(|a| a.id == id).cloned()
    }

    // ── Internal helpers ────────────────────────────────────────

    /// Push simplified config to a connected agent (services + power_policy).
    async fn push_config_to_agent(&self, app: &Application) {
        let conns = self.connections.read().await;
        let Some(conn) = conns.get(&app.id) else {
            return;
        };

        let _ = conn
            .tx
            .send(RegistryMessage::Config {
                config_version: 1,
                services: app.services.clone(),
                power_policy: app.power_policy.clone(),
                base_domain: self.env.base_domain.clone(),
                slug: app.slug.clone(),
                frontend: Some(app.frontend.clone()),
                apis: app.apis.clone(),
                code_server_enabled: app.code_server_enabled,
                wake_page_enabled: app.wake_page_enabled,
            })
            .await;
    }

    // ── Service control & metrics ──────────────────────────────────

    /// Send a service start/stop command to a connected agent.
    pub async fn send_service_command(
        &self,
        app_id: &str,
        service_type: ServiceType,
        action: ServiceAction,
    ) -> Result<bool> {
        let conns = self.connections.read().await;
        let Some(conn) = conns.get(app_id) else {
            return Ok(false);
        };

        conn.tx
            .send(RegistryMessage::ServiceCommand {
                service_type,
                action,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Failed to send command to agent"))?;

        info!(
            app_id,
            service_type = ?service_type,
            action = ?action,
            "Service command sent to agent"
        );
        Ok(true)
    }

    /// Update power policy for an application and push to connected agent.
    pub async fn update_power_policy(&self, app_id: &str, policy: PowerPolicy) -> Result<bool> {
        // Update in state
        {
            let mut state = self.state.write().await;
            let Some(app) = state.applications.iter_mut().find(|a| a.id == app_id) else {
                return Ok(false);
            };
            app.power_policy = policy.clone();
        }
        self.persist().await?;

        // Push to connected agent
        let conns = self.connections.read().await;
        if let Some(conn) = conns.get(app_id) {
            let _ = conn
                .tx
                .send(RegistryMessage::PowerPolicyUpdate(policy))
                .await;
            info!(app_id, "Power policy update sent to agent");
        }

        Ok(true)
    }

    /// Handle metrics received from an agent: update in-memory state and broadcast to WebSocket.
    pub async fn handle_metrics(&self, app_id: &str, metrics: AgentMetrics) {
        // Convert ServiceState to string for broadcast
        let code_server_status = service_state_str(metrics.code_server_status);
        let app_status = service_state_str(metrics.app_status);
        let db_status = service_state_str(metrics.db_status);

        // Update in-memory metrics (not persisted)
        {
            let mut state = self.state.write().await;
            if let Some(app) = state.applications.iter_mut().find(|a| a.id == app_id) {
                app.metrics = Some(metrics.clone());
            }
        }

        // Broadcast to WebSocket
        let _ = self.events.agent_metrics.send(AgentMetricsEvent {
            app_id: app_id.to_string(),
            code_server_status,
            app_status,
            db_status,
            memory_bytes: metrics.memory_bytes,
            cpu_percent: metrics.cpu_percent,
            code_server_idle_secs: metrics.code_server_idle_secs,
        });
    }

    /// Handle an IP update from an agent (e.g. after container restart with new DHCP lease).
    /// Updates the stored IPv4 address and pushes a Config refresh so the agent re-publishes routes.
    pub async fn handle_ip_update(&self, app_id: &str, ipv4_str: &str) {
        let updated_app = {
            let mut state = self.state.write().await;
            if let Some(app) = state.applications.iter_mut().find(|a| a.id == app_id) {
                if let Ok(addr) = ipv4_str.parse() {
                    let old = app.ipv4_address;
                    app.ipv4_address = Some(addr);
                    info!(app_id, old = ?old, new = ipv4_str, "Updated app IPv4 from agent IP update");
                    Some(app.clone())
                } else {
                    warn!(app_id, ipv4_str, "Invalid IPv4 in IpUpdate");
                    None
                }
            } else {
                None
            }
        };

        if let Some(app) = updated_app {
            let _ = self.persist().await;
            // Push fresh Config so the agent re-publishes routes with the new IP
            self.push_config_to_agent(&app).await;
        }
    }

    /// Handle schema metadata received from an agent: cache and broadcast to WebSocket.
    pub async fn handle_schema_metadata(
        &self,
        app_id: &str,
        tables: Vec<crate::protocol::SchemaTableInfo>,
        relations: Vec<crate::protocol::SchemaRelationInfo>,
        version: u64,
        _db_size_bytes: u64,
    ) {
        // Look up the app slug
        let slug = {
            let state = self.state.read().await;
            state.applications.iter()
                .find(|a| a.id == app_id)
                .map(|a| a.slug.clone())
                .unwrap_or_default()
        };

        // Broadcast to WebSocket
        use hr_common::events::{DataverseSchemaEvent, DataverseTableSummary};
        let table_summaries: Vec<DataverseTableSummary> = tables.iter().map(|t| {
            DataverseTableSummary {
                name: t.name.clone(),
                slug: t.slug.clone(),
                columns_count: t.columns.len(),
                rows_count: t.row_count,
            }
        }).collect();

        let _ = self.events.dataverse_schema.send(DataverseSchemaEvent {
            app_id: app_id.to_string(),
            slug,
            tables: table_summaries,
            relations_count: relations.len(),
            version,
        });
    }

    /// Proxy a Dataverse query to an agent and wait for the result.
    pub async fn dataverse_query(
        &self,
        app_id: &str,
        query: crate::protocol::DataverseQueryRequest,
    ) -> Result<serde_json::Value> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.dataverse_query_signals.write().await.insert(request_id.clone(), tx);

        // Send the query to the agent
        let connections = self.connections.read().await;
        let conn = connections.get(app_id)
            .ok_or_else(|| anyhow::anyhow!("Agent not connected for app {}", app_id))?;
        conn.tx.send(RegistryMessage::DataverseQuery {
            request_id: request_id.clone(),
            query,
        }).await.map_err(|_| anyhow::anyhow!("Failed to send query to agent"))?;
        drop(connections);

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(Ok(data))) => Ok(data),
            Ok(Ok(Err(e))) => anyhow::bail!("Dataverse query error: {}", e),
            Ok(Err(_)) => {
                self.dataverse_query_signals.write().await.remove(&request_id);
                anyhow::bail!("Dataverse query channel closed")
            }
            Err(_) => {
                self.dataverse_query_signals.write().await.remove(&request_id);
                anyhow::bail!("Dataverse query timeout after 30s")
            }
        }
    }

    /// Handle a Dataverse query result from an agent.
    pub async fn on_dataverse_query_result(
        &self,
        request_id: &str,
        data: Option<serde_json::Value>,
        error: Option<String>,
    ) {
        if let Some(tx) = self.dataverse_query_signals.write().await.remove(request_id) {
            let result = match error {
                Some(e) => Err(e),
                None => Ok(data.unwrap_or(serde_json::Value::Null)),
            };
            let _ = tx.send(result);
        }
    }

    /// Send a RegistryMessage to a connected agent by app_id.
    pub async fn send_to_agent(&self, app_id: &str, msg: RegistryMessage) -> Result<()> {
        let connections = self.connections.read().await;
        let conn = connections.get(app_id)
            .ok_or_else(|| anyhow::anyhow!("Agent not connected for app {}", app_id))?;
        conn.tx.send(msg).await.map_err(|_| anyhow::anyhow!("Failed to send to agent"))?;
        Ok(())
    }

    /// Handle service state changed event from agent (broadcasts to WebSocket).
    pub fn handle_service_state_changed(
        &self,
        app_id: &str,
        service_type: ServiceType,
        new_state: ServiceState,
    ) {
        use hr_common::events::ServiceCommandEvent;

        let action = match new_state {
            ServiceState::Running => "started",
            ServiceState::Stopped | ServiceState::ManuallyOff => "stopped",
            ServiceState::Starting => "starting",
            ServiceState::Stopping => "stopping",
        };

        let _ = self.events.service_command.send(ServiceCommandEvent {
            app_id: app_id.to_string(),
            service_type: service_type_str(service_type).to_string(),
            action: action.to_string(),
            success: true,
        });
    }

    /// Send an activity ping to a connected agent to keep powersave alive.
    pub async fn send_activity_ping(&self, app_id: &str, service_type: ServiceType) {
        let connections = self.connections.read().await;
        if let Some(conn) = connections.get(app_id) {
            let _ = conn.tx.send(RegistryMessage::ActivityPing { service_type }).await;
        }
    }

    // ── Agent Update ────────────────────────────────────────────────

    /// Trigger update to specified agents (or all connected if None).
    /// Sends `UpdateAvailable` message to each agent with the current binary info.
    pub async fn trigger_update(
        &self,
        agent_ids: Option<Vec<String>>,
    ) -> Result<UpdateBatchResult> {
        use ring::digest::{Context, SHA256};
        use std::io::Read;

        // Read current binary and compute SHA256
        let binary_path = Path::new("/opt/homeroute/data/agent-binaries/hr-agent");
        if !binary_path.exists() {
            anyhow::bail!("Agent binary not found at {}", binary_path.display());
        }

        let metadata = std::fs::metadata(binary_path)?;
        let modified = metadata
            .modified()
            .map(|t| {
                let dt: DateTime<Utc> = t.into();
                dt.format("%Y%m%d-%H%M%S").to_string()
            })
            .unwrap_or_else(|_| "unknown".to_string());

        let mut file = std::fs::File::open(binary_path)?;
        let mut context = Context::new(&SHA256);
        let mut buffer = [0u8; 8192];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            context.update(&buffer[..count]);
        }
        let sha256 = hex::encode(context.finish().as_ref());

        let download_url = format!(
            "http://10.0.0.254:{}/api/applications/agents/binary",
            self.env.api_port
        );

        let state = self.state.read().await;
        let conns = self.connections.read().await;

        let mut notified = Vec::new();
        let mut skipped = Vec::new();

        // Determine which apps to target
        let target_ids: Vec<&str> = match &agent_ids {
            Some(ids) => ids.iter().map(|s| s.as_str()).collect(),
            None => conns.keys().map(|s| s.as_str()).collect(),
        };

        for app in &state.applications {
            if !target_ids.contains(&app.id.as_str()) {
                continue;
            }

            if let Some(conn) = conns.get(&app.id) {
                let msg = RegistryMessage::UpdateAvailable {
                    version: modified.clone(),
                    download_url: download_url.clone(),
                    sha256: sha256.clone(),
                };

                if conn.tx.send(msg).await.is_ok() {
                    notified.push(AgentNotifyResult {
                        id: app.id.clone(),
                        slug: app.slug.clone(),
                        status: "notified".to_string(),
                    });

                    // Emit event
                    let _ = self.events.agent_update.send(AgentUpdateEvent {
                        app_id: app.id.clone(),
                        slug: app.slug.clone(),
                        status: AgentUpdateStatus::Notified,
                        version: Some(modified.clone()),
                        error: None,
                    });

                    info!(app = app.slug, version = modified, "Update notification sent");
                } else {
                    skipped.push(AgentSkipResult {
                        id: app.id.clone(),
                        slug: app.slug.clone(),
                        reason: "send_failed".to_string(),
                    });
                }
            } else {
                skipped.push(AgentSkipResult {
                    id: app.id.clone(),
                    slug: app.slug.clone(),
                    reason: "not_connected".to_string(),
                });
            }
        }

        info!(
            notified = notified.len(),
            skipped = skipped.len(),
            version = modified,
            "Agent update triggered"
        );

        Ok(UpdateBatchResult {
            version: modified,
            sha256,
            agents_notified: notified,
            agents_skipped: skipped,
        })
    }

    /// Get update status for all agents: whether they're connected with the expected version.
    pub async fn get_update_status(&self) -> Result<UpdateStatusResult> {
        use ring::digest::{Context, SHA256};
        use std::io::Read;

        // Get expected version from current binary
        let binary_path = Path::new("/opt/homeroute/data/agent-binaries/hr-agent");
        let expected_version = if binary_path.exists() {
            std::fs::metadata(binary_path)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| {
                    let dt: DateTime<Utc> = t.into();
                    dt.format("%Y%m%d-%H%M%S").to_string()
                })
                .unwrap_or_else(|| "unknown".to_string())
        } else {
            "no_binary".to_string()
        };

        let state = self.state.read().await;
        let conns = self.connections.read().await;
        let now = Utc::now();

        let agents: Vec<AgentUpdateStatusInfo> = state
            .applications
            .iter()
            .map(|app| {
                let is_connected = conns.contains_key(&app.id);
                let version_matches = app
                    .agent_version
                    .as_ref()
                    .map(|v| v == &expected_version)
                    .unwrap_or(false);
                let has_recent_heartbeat = app
                    .last_heartbeat
                    .map(|hb| now - hb < chrono::Duration::seconds(90))
                    .unwrap_or(false);

                let update_status = if !is_connected {
                    "disconnected"
                } else if version_matches {
                    "success"
                } else {
                    "pending"
                };

                AgentUpdateStatusInfo {
                    id: app.id.clone(),
                    slug: app.slug.clone(),
                    container_name: app.container_name.clone(),
                    status: if is_connected {
                        "connected"
                    } else {
                        "disconnected"
                    }
                    .to_string(),
                    current_version: app.agent_version.clone(),
                    update_status: update_status.to_string(),
                    metrics_flowing: is_connected && has_recent_heartbeat,
                    last_heartbeat: app.last_heartbeat,
                }
            })
            .collect();

        Ok(UpdateStatusResult {
            expected_version,
            agents,
        })
    }

    /// Fix a failed agent update via machinectl exec (fallback mechanism).
    /// Downloads the binary directly in the container and restarts the agent.
    pub async fn fix_agent_via_exec(&self, app_id: &str) -> Result<String> {
        let (container, slug) = {
            let state = self.state.read().await;
            let app = state
                .applications
                .iter()
                .find(|a| a.id == app_id)
                .ok_or_else(|| anyhow::anyhow!("Application not found: {}", app_id))?;
            (app.container_name.clone(), app.slug.clone())
        };

        let api_port = self.env.api_port;

        info!(container = container, slug = slug, "Fixing agent via machinectl exec");

        // Download new binary directly in the container and restart
        let download_cmd = format!(
            "curl -fsSL http://10.0.0.254:{}/api/applications/agents/binary -o /usr/local/bin/hr-agent.new && \
             chmod +x /usr/local/bin/hr-agent.new && \
             mv /usr/local/bin/hr-agent.new /usr/local/bin/hr-agent && \
             systemctl restart hr-agent",
            api_port
        );

        let output = tokio::process::Command::new("machinectl")
            .args(["shell", &container, "/bin/bash", "-c", &download_cmd])
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("machinectl exec failed: {e}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            anyhow::bail!("machinectl exec failed: {}", stderr);
        }

        info!(container = container, "Agent fixed via machinectl exec");

        // Emit event
        let _ = self.events.agent_update.send(AgentUpdateEvent {
            app_id: app_id.to_string(),
            slug: slug.to_string(),
            status: AgentUpdateStatus::Notified,
            version: None,
            error: None,
        });

        Ok(if stdout.is_empty() { stderr } else { stdout })
    }

    /// Periodic cleanup of stale migration signals and transfer mappings.
    pub async fn cleanup_stale_signals(&self) {
        {
            let mut signals = self.migration_signals.write().await;
            let before = signals.len();
            signals.retain(|_tid, tx| !tx.is_closed());
            let removed = before - signals.len();
            if removed > 0 {
                tracing::info!("Cleaned up {} stale migration signals", removed);
            }
        }
        {
            let mut signals = self.exec_signals.write().await;
            let before = signals.len();
            signals.retain(|_rid, tx| !tx.is_closed());
            let removed = before - signals.len();
            if removed > 0 {
                tracing::info!("Cleaned up {} stale exec signals", removed);
            }
        }
        {
            let signal_keys: std::collections::HashSet<String> = self.migration_signals.read().await.keys().cloned().collect();
            let mut names = self.transfer_container_names.write().await;
            let before = names.len();
            names.retain(|tid, _| signal_keys.contains(tid));
            let removed = before - names.len();
            if removed > 0 {
                tracing::info!("Cleaned up {} stale transfer container name mappings", removed);
            }
        }
        {
            let signal_keys: std::collections::HashSet<String> = self.migration_signals.read().await.keys().cloned().collect();
            let mut relays = self.transfer_relay_targets.write().await;
            let before = relays.len();
            relays.retain(|tid, _| signal_keys.contains(tid));
            let removed = before - relays.len();
            if removed > 0 {
                tracing::info!("Cleaned up {} stale transfer relay target mappings", removed);
            }
        }
        {
            let mut signals = self.dataverse_query_signals.write().await;
            let before = signals.len();
            signals.retain(|_rid, tx| !tx.is_closed());
            let removed = before - signals.len();
            if removed > 0 {
                tracing::info!("Cleaned up {} stale dataverse query signals", removed);
            }
        }
    }

    // ── Terminal session management ────────────────────────────

    /// Register a terminal session so data from a host-agent can be routed to the API WS handler.
    pub async fn register_terminal_session(&self, session_id: &str, tx: mpsc::Sender<Vec<u8>>) {
        self.terminal_sessions.write().await.insert(session_id.to_string(), tx);
    }

    /// Unregister a terminal session.
    pub async fn unregister_terminal_session(&self, session_id: &str) {
        self.terminal_sessions.write().await.remove(session_id);
    }

    /// Forward terminal data from a host-agent to the registered API WS handler.
    pub async fn send_terminal_data(&self, session_id: &str, data: Vec<u8>) {
        let sessions = self.terminal_sessions.read().await;
        if let Some(tx) = sessions.get(session_id) {
            let _ = tx.send(data).await;
        }
    }

    /// Persist state to disk (atomic write).
    async fn persist(&self) -> Result<()> {
        let state = self.state.read().await;
        self.persist_inner(&state).await
    }

    async fn persist_inner(&self, state: &RegistryState) -> Result<()> {
        let json = serde_json::to_string_pretty(state)?;
        let tmp = self.state_path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &json).await?;
        tokio::fs::rename(&tmp, &self.state_path).await?;
        Ok(())
    }
}

// ── Token helpers ───────────────────────────────────────────────

fn generate_token() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    hex::encode(bytes)
}

fn hash_token(token: &str) -> Result<String> {
    use argon2::{Argon2, PasswordHasher};
    use argon2::password_hash::SaltString;
    use rand_core::OsRng;

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(token.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash failed: {e}"))?;
    Ok(hash.to_string())
}

fn verify_token(token: &str, hash: &str) -> bool {
    use argon2::{Argon2, PasswordVerifier};
    use argon2::password_hash::PasswordHash;

    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(token.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_roundtrip() {
        let token = generate_token();
        assert_eq!(token.len(), 64);
        let hash = hash_token(&token).unwrap();
        assert!(verify_token(&token, &hash));
        assert!(!verify_token("wrong", &hash));
    }
}
