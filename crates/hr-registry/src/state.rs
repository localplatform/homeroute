//! Agent registry: manages application lifecycle, agent connections,
//! and pushes config to agents.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

use hr_common::config::EnvConfig;
use hr_common::events::{AgentMetricsEvent, AgentStatusEvent, AgentUpdateEvent, AgentUpdateStatus, EventBus};
use hr_lxd::LxdClient;

use crate::protocol::{AgentMetrics, ContainerInfo, HostMetrics, HostRegistryMessage, PowerPolicy, RegistryMessage, ServiceAction, ServiceState, ServiceType};
use crate::types::{
    AgentNotifyResult, AgentSkipResult, AgentStatus, AgentUpdateStatusInfo,
    Application, CreateApplicationRequest, RegistryState, UpdateApplicationRequest,
    UpdateBatchResult, UpdateStatusResult,
};

/// An active agent connection (in-memory only).
struct AgentConnection {
    tx: mpsc::Sender<RegistryMessage>,
    connected_at: DateTime<Utc>,
    last_heartbeat: DateTime<Utc>,
}

/// In-memory host-agent connection state.
pub struct HostConnection {
    pub tx: mpsc::Sender<HostRegistryMessage>,
    pub host_name: String,
    pub connected_at: DateTime<Utc>,
    pub last_heartbeat: DateTime<Utc>,
    pub version: Option<String>,
    pub metrics: Option<HostMetrics>,
    pub containers: Vec<ContainerInfo>,
}

pub enum MigrationResult {
    ImportComplete { container_name: String },
    ImportFailed { error: String },
    ExportFailed { error: String },
}

pub struct AgentRegistry {
    state: Arc<RwLock<RegistryState>>,
    state_path: PathBuf,
    connections: Arc<RwLock<HashMap<String, AgentConnection>>>,
    pub host_connections: Arc<RwLock<HashMap<String, HostConnection>>>,
    env: Arc<EnvConfig>,
    events: Arc<EventBus>,
    migration_signals: Arc<RwLock<HashMap<String, tokio::sync::oneshot::Sender<MigrationResult>>>>,
    exec_signals: Arc<RwLock<HashMap<String, tokio::sync::oneshot::Sender<(bool, String, String)>>>>,
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
        }
    }

    // ── Application CRUD ────────────────────────────────────────

    /// Create a new application: generates token, saves record immediately,
    /// then deploys LXC container + agent in a background task.
    /// Returns the application (status=deploying) and the cleartext token.
    pub async fn create_application(
        self: &Arc<Self>,
        req: CreateApplicationRequest,
    ) -> Result<(Application, String)> {
        // Generate token
        let token_clear = generate_token();
        let token_hash = hash_token(&token_clear)?;

        let id = uuid::Uuid::new_v4().to_string();
        let container_name = format!("hr-{}", req.slug);

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
            metrics: None,
        };

        // Store in state immediately so the UI can see the app
        {
            let mut state = self.state.write().await;
            state.applications.push(app.clone());
        }
        self.persist().await?;

        info!(app = app.slug, container = container_name, "Application created, starting background deploy");

        // Spawn background deploy task
        let registry = Arc::clone(self);
        let token_for_deploy = token_clear.clone();
        let slug = app.slug.clone();
        let app_id = id.clone();
        tokio::spawn(async move {
            registry.run_deploy_background(&app_id, &slug, &container_name, &token_for_deploy).await;
        });

        Ok((app, token_clear))
    }

    /// Background deployment: creates LXC container, deploys agent, emits progress events.
    async fn run_deploy_background(
        &self,
        app_id: &str,
        slug: &str,
        container_name: &str,
        token: &str,
    ) {
        let emit = |message: &str| {
            let _ = self.events.agent_status.send(AgentStatusEvent {
                app_id: app_id.to_string(),
                slug: slug.to_string(),
                status: "deploying".to_string(),
                message: Some(message.to_string()),
            });
        };

        emit("Creation du conteneur LXC...");

        // Create the LXC container
        if let Err(e) = LxdClient::create_container(container_name).await {
            error!(container = container_name, "LXC creation failed: {e}");
            emit(&format!("Erreur: {e}"));
            self.set_app_status(app_id, AgentStatus::Error).await;
            // Remove the app from state on failure
            self.remove_failed_app(app_id).await;
            return;
        }

        // Deploy hr-agent into the container
        if let Err(e) = self.deploy_agent(container_name, slug, token, &emit).await {
            error!(container = container_name, "Agent deploy failed: {e}");
            emit(&format!("Erreur: {e}"));
            self.set_app_status(app_id, AgentStatus::Error).await;
            // Cleanup container on failure
            let _ = LxdClient::delete_container(container_name).await;
            self.remove_failed_app(app_id).await;
            return;
        }

        // Update status to pending only if agent hasn't already connected
        {
            let mut state = self.state.write().await;
            if let Some(app) = state.applications.iter_mut().find(|a| a.id == app_id) {
                if app.status == AgentStatus::Deploying {
                    app.status = AgentStatus::Pending;
                }
            }
        }
        let _ = self.persist().await;

        let _ = self.events.agent_status.send(AgentStatusEvent {
            app_id: app_id.to_string(),
            slug: slug.to_string(),
            status: "pending".to_string(),
            message: Some("Deploiement termine".to_string()),
        });

        info!(app = slug, container = container_name, "Background deploy complete");
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

        let app = app.clone();
        drop(state);

        self.persist().await?;

        // Push new config to connected agent if any
        self.push_config_to_agent(&app).await;

        Ok(Some(app))
    }

    /// Remove an application: disconnect agent and delete LXC container.
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

        // Delete LXC container
        if let Err(e) = LxdClient::delete_container(&app.container_name).await {
            warn!(container = app.container_name, "Failed to delete container: {e}");
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

        // Store connection
        {
            let mut conns = self.connections.write().await;
            conns.insert(
                app_id.to_string(),
                AgentConnection {
                    tx: tx.clone(),
                    connected_at: now,
                    last_heartbeat: now,
                },
            );
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

        // Push simplified config
        if let Some(app) = app {
            let _ = tx
                .send(RegistryMessage::Config {
                    config_version: 1,
                    services: app.services.clone(),
                    power_policy: app.power_policy.clone(),
                })
                .await;
        }

        self.persist().await?;
        Ok(())
    }

    /// Called when an agent WebSocket disconnects.
    pub async fn on_agent_disconnected(&self, app_id: &str) {
        {
            let mut conns = self.connections.write().await;
            conns.remove(app_id);
        }

        {
            let mut state = self.state.write().await;
            if let Some(app) = state.applications.iter_mut().find(|a| a.id == app_id) {
                app.status = AgentStatus::Disconnected;
            }
        }

        let _ = self.persist().await;
        info!(app_id, "Agent disconnected");
    }

    /// Update heartbeat timestamp for an agent.
    pub async fn handle_heartbeat(&self, app_id: &str) {
        let now = Utc::now();
        {
            let mut conns = self.connections.write().await;
            if let Some(conn) = conns.get_mut(app_id) {
                conn.last_heartbeat = now;
            }
        }
        {
            let mut state = self.state.write().await;
            if let Some(app) = state.applications.iter_mut().find(|a| a.id == app_id) {
                app.last_heartbeat = Some(now);
            }
        }
    }

    /// Background task: check heartbeats and mark stale agents as disconnected.
    pub async fn run_heartbeat_monitor(self: &Arc<Self>) {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;

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
                warn!(app_id = id, "Agent heartbeat stale, marking disconnected");
                self.on_agent_disconnected(&id).await;
            }
        }
    }

    // ── Host-agent connection lifecycle ────────────────────────

    pub async fn on_host_connected(
        &self,
        host_id: String,
        host_name: String,
        tx: mpsc::Sender<HostRegistryMessage>,
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
        };
        self.host_connections.write().await.insert(host_id.clone(), conn);
        info!("Host agent connected: {} ({})", host_name, host_id);
    }

    pub async fn on_host_disconnected(&self, host_id: &str) {
        if let Some(conn) = self.host_connections.write().await.remove(host_id) {
            info!("Host agent disconnected: {} ({})", conn.host_name, host_id);
        }
    }

    pub async fn is_host_connected(&self, host_id: &str) -> bool {
        self.host_connections.read().await.contains_key(host_id)
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

    pub async fn send_host_command(
        &self,
        host_id: &str,
        msg: HostRegistryMessage,
    ) -> Result<(), String> {
        let conns = self.host_connections.read().await;
        match conns.get(host_id) {
            Some(conn) => conn
                .tx
                .send(msg)
                .await
                .map_err(|e| format!("Failed to send to host {}: {}", host_id, e)),
            None => Err(format!("Host {} not connected", host_id)),
        }
    }

    // ── Migration & exec signal handling ──────────────────────

    pub async fn register_migration_signal(&self, transfer_id: &str) -> tokio::sync::oneshot::Receiver<MigrationResult> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.migration_signals.write().await.insert(transfer_id.to_string(), tx);
        rx
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

    /// Deploy the hr-agent binary and config into an LXC container.
    /// `emit` is called with progress messages for real-time UI updates.
    async fn deploy_agent(
        &self,
        container: &str,
        service_name: &str,
        token: &str,
        emit: impl Fn(&str),
    ) -> Result<()> {
        let agent_binary = PathBuf::from("/opt/homeroute/data/agent-binaries/hr-agent");
        if !agent_binary.exists() {
            anyhow::bail!(
                "Agent binary not found at {}. Build it first with: cargo build --release -p hr-agent",
                agent_binary.display()
            );
        }

        // Push binary
        emit("Deploiement du binaire agent...");
        LxdClient::push_file(container, &agent_binary, "usr/local/bin/hr-agent").await?;
        LxdClient::exec(container, &["chmod", "+x", "/usr/local/bin/hr-agent"]).await?;

        // Generate config TOML
        emit("Configuration de l'agent...");
        let api_port = self.env.api_port;
        let config_content = format!(
            r#"homeroute_address = "10.0.0.254"
homeroute_port = {api_port}
token = "{token}"
service_name = "{service_name}"
interface = "eth0"
"#
        );

        let tmp_config = PathBuf::from(format!("/tmp/hr-agent-{service_name}.toml"));
        tokio::fs::write(&tmp_config, &config_content).await?;
        LxdClient::push_file(container, &tmp_config, "etc/hr-agent.toml").await?;
        let _ = tokio::fs::remove_file(&tmp_config).await;

        // Push systemd unit
        let unit_content = r#"[Unit]
Description=HomeRoute Agent
After=network.target

[Service]
ExecStart=/usr/local/bin/hr-agent
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
"#;
        let tmp_unit = PathBuf::from(format!("/tmp/hr-agent-{service_name}.service"));
        tokio::fs::write(&tmp_unit, unit_content).await?;
        LxdClient::push_file(container, &tmp_unit, "etc/systemd/system/hr-agent.service").await?;
        let _ = tokio::fs::remove_file(&tmp_unit).await;

        // Enable and start agent
        emit("Demarrage de l'agent...");
        LxdClient::exec(container, &["systemctl", "daemon-reload"]).await?;
        LxdClient::exec(container, &["systemctl", "enable", "--now", "hr-agent"]).await?;

        // Wait for network connectivity before installing packages
        emit("Attente de la connectivite reseau...");
        LxdClient::wait_for_network(container, 30).await?;

        // Install code-server dependencies with retry
        emit("Installation des dependances...");
        LxdClient::exec_with_retry(
            container,
            &["bash", "-c", "apt-get update -qq && apt-get install -y -qq curl"],
            3,
        )
        .await
        .with_context(|| format!("Failed to install curl in {container}"))?;

        emit("Installation de code-server...");
        LxdClient::exec_with_retry(
            container,
            &["bash", "-c", "curl -fsSL https://code-server.dev/install.sh | sh -s -- --method=standalone --prefix=/usr/local"],
            3,
        )
        .await
        .with_context(|| format!("Failed to install code-server in {container}"))?;

        // Attach a separate storage volume for the workspace (independent of boot disk)
        emit("Creation du volume workspace...");
        let vol_name = format!("{container}-workspace");
        LxdClient::attach_storage_volume(container, &vol_name, "/root/workspace")
            .await
            .with_context(|| format!("Failed to attach workspace volume for {container}"))?;

        // Configure code-server: no auth (forward-auth handles it), bind localhost
        emit("Configuration de code-server...");
        LxdClient::exec(container, &["mkdir", "-p", "/root/.config/code-server"]).await?;
        let cs_config = "bind-addr: 127.0.0.1:13337\nauth: none\ncert: false\n";
        let tmp_cs_config = PathBuf::from(format!("/tmp/cs-config-{service_name}.yaml"));
        tokio::fs::write(&tmp_cs_config, cs_config).await?;
        LxdClient::push_file(container, &tmp_cs_config, "root/.config/code-server/config.yaml").await?;
        let _ = tokio::fs::remove_file(&tmp_cs_config).await;

        // VS Code settings: dark theme, disable built-in AI features, disable auto port forwarding
        LxdClient::exec(container, &["mkdir", "-p", "/root/.local/share/code-server/User"]).await?;
        let cs_settings = r#"{
  "workbench.colorTheme": "Default Dark Modern",
  "chat.disableAIFeatures": true,
  "workbench.startupEditor": "none",
  "telemetry.telemetryLevel": "off",
  "remote.autoForwardPorts": false
}
"#;
        let tmp_cs_settings = PathBuf::from(format!("/tmp/cs-settings-{service_name}.json"));
        tokio::fs::write(&tmp_cs_settings, cs_settings).await?;
        LxdClient::push_file(container, &tmp_cs_settings, "root/.local/share/code-server/User/settings.json").await?;
        let _ = tokio::fs::remove_file(&tmp_cs_settings).await;

        // Create systemd service for code-server (opens /root/workspace by default)
        // Extension install runs as a one-shot service in the background to avoid blocking deploy
        let cs_unit = r#"[Unit]
Description=code-server IDE
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/code-server --bind-addr 127.0.0.1:13337 /root/workspace
Restart=always
RestartSec=5
Environment=HOME=/root

# Ensure all child processes (extensions, LSP, file watchers) are killed on stop
KillMode=control-group
KillSignal=SIGTERM
TimeoutStopSec=10

[Install]
WantedBy=multi-user.target
"#;
        let tmp_cs_unit = PathBuf::from(format!("/tmp/cs-unit-{service_name}.service"));
        tokio::fs::write(&tmp_cs_unit, cs_unit).await?;
        LxdClient::push_file(container, &tmp_cs_unit, "etc/systemd/system/code-server.service").await?;
        let _ = tokio::fs::remove_file(&tmp_cs_unit).await;

        // One-shot service to install/update Claude Code extension on every boot
        // Uninstalls first to ensure latest version is always fetched
        let cs_setup_unit = r#"[Unit]
Description=code-server Claude Code extension updater
After=network-online.target code-server.service
Wants=network-online.target

[Service]
Type=oneshot
ExecStartPre=-/usr/local/bin/code-server --uninstall-extension Anthropic.claude-code
ExecStart=/usr/local/bin/code-server --install-extension Anthropic.claude-code
RemainAfterExit=true
Environment=HOME=/root

[Install]
WantedBy=multi-user.target
"#;
        let tmp_cs_setup = PathBuf::from(format!("/tmp/cs-setup-{service_name}.service"));
        tokio::fs::write(&tmp_cs_setup, cs_setup_unit).await?;
        LxdClient::push_file(container, &tmp_cs_setup, "etc/systemd/system/code-server-setup.service").await?;
        let _ = tokio::fs::remove_file(&tmp_cs_setup).await;

        emit("Demarrage de code-server...");
        LxdClient::exec(container, &["systemctl", "daemon-reload"]).await?;
        LxdClient::exec(container, &["systemctl", "enable", "--now", "code-server"]).await?;
        LxdClient::exec(container, &["systemctl", "enable", "--now", "code-server-setup"]).await?;
        info!(container, "code-server installed and started");

        info!(container, "Agent deployed");
        Ok(())
    }

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
        let code_server_status = format!("{:?}", metrics.code_server_status).to_lowercase();
        let app_status = format!("{:?}", metrics.app_status).to_lowercase();
        let db_status = format!("{:?}", metrics.db_status).to_lowercase();

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
            app_idle_secs: metrics.app_idle_secs,
        });
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
            service_type: format!("{:?}", service_type).to_lowercase(),
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

    /// Fix a failed agent update via LXC exec (fallback mechanism).
    /// Downloads the binary directly in the container and restarts the agent.
    pub async fn fix_agent_via_lxc(&self, app_id: &str) -> Result<String> {
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

        info!(container = container, slug = slug, "Fixing agent via LXC exec");

        // Download new binary directly in the container and restart
        let download_cmd = format!(
            "curl -fsSL http://10.0.0.254:{}/api/applications/agents/binary -o /usr/local/bin/hr-agent.new && \
             chmod +x /usr/local/bin/hr-agent.new && \
             mv /usr/local/bin/hr-agent.new /usr/local/bin/hr-agent && \
             systemctl restart hr-agent",
            api_port
        );

        let output = LxdClient::exec(&container, &["bash", "-c", &download_cmd]).await?;

        info!(container = container, "Agent fixed via LXC exec");

        // Emit event
        let _ = self.events.agent_update.send(AgentUpdateEvent {
            app_id: app_id.to_string(),
            slug: slug.to_string(),
            status: AgentUpdateStatus::Notified,
            version: None,
            error: None,
        });

        Ok(output)
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
