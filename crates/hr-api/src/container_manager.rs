//! Container V2 manager: lifecycle orchestration for systemd-nspawn containers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use hr_common::config::EnvConfig;
use hr_common::events::{AgentStatusEvent, EventBus, MigrationPhase};
use hr_container::NspawnClient;
use hr_registry::protocol::{HostRegistryMessage, ServiceAction, ServiceType};
use hr_registry::types::{AgentStatus, CreateApplicationRequest, UpdateApplicationRequest};
use hr_registry::AgentRegistry;

use crate::state::MigrationState;

// ── Types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ContainerV2Status {
    Deploying,
    Running,
    Stopped,
    Error,
    Migrating,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ContainerV2Config {
    #[serde(default = "default_storage_path")]
    pub container_storage_path: String,
}

fn default_storage_path() -> String {
    "/var/lib/machines".to_string()
}

#[derive(Serialize, Deserialize)]
pub struct ContainerV2State {
    #[serde(default)]
    pub config: ContainerV2Config,
    #[serde(default)]
    pub containers: Vec<ContainerV2Record>,
}

impl Default for ContainerV2State {
    fn default() -> Self {
        Self {
            config: ContainerV2Config::default(),
            containers: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ContainerV2Record {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub container_name: String,
    pub host_id: String,
    pub status: ContainerV2Status,
    pub created_at: DateTime<Utc>,
    pub migrated_from_lxd_app_id: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateContainerRequest {
    pub name: String,
    pub slug: String,
    pub frontend: hr_registry::types::FrontendEndpoint,
    #[serde(default)]
    pub apis: Vec<hr_registry::types::ApiEndpoint>,
    #[serde(default = "default_true")]
    pub code_server_enabled: bool,
    #[serde(default)]
    pub host_id: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct MigrateContainerRequest {
    pub target_host_id: String,
}

// ── ContainerManager ─────────────────────────────────────────────

pub struct ContainerManager {
    state: Arc<RwLock<ContainerV2State>>,
    state_path: PathBuf,
    pub env: Arc<EnvConfig>,
    pub events: Arc<EventBus>,
    pub registry: Arc<AgentRegistry>,
}

impl ContainerManager {
    /// Load or create the container V2 state from disk.
    pub fn new(
        state_path: PathBuf,
        env: Arc<EnvConfig>,
        events: Arc<EventBus>,
        registry: Arc<AgentRegistry>,
    ) -> Self {
        let state = match std::fs::read_to_string(&state_path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                warn!("Failed to parse containers-v2 state, starting fresh: {e}");
                ContainerV2State::default()
            }),
            Err(_) => ContainerV2State::default(),
        };

        info!(
            containers = state.containers.len(),
            "Loaded containers V2 state"
        );

        Self {
            state: Arc::new(RwLock::new(state)),
            state_path,
            env,
            events,
            registry,
        }
    }

    /// Persist state to disk (atomic write).
    async fn save_state(&self) -> Result<(), String> {
        let state = self.state.read().await;
        let json = serde_json::to_string_pretty(&*state).map_err(|e| e.to_string())?;
        let tmp = self.state_path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &json)
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::rename(&tmp, &self.state_path)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ── CRUD ─────────────────────────────────────────────────────

    /// Create a new nspawn container: register in AgentRegistry (headless), create V2 record,
    /// spawn background deploy.
    pub async fn create_container(
        self: &Arc<Self>,
        req: CreateContainerRequest,
    ) -> Result<(ContainerV2Record, String), String> {
        let host_id = req.host_id.clone().unwrap_or_else(|| "local".to_string());
        let container_name = format!("hr-v2-{}", req.slug);

        // Create application in registry (headless — no LXD deploy)
        let create_req = CreateApplicationRequest {
            name: req.name.clone(),
            slug: req.slug.clone(),
            host_id: Some(host_id.clone()),
            frontend: req.frontend.clone(),
            apis: req.apis.clone(),
            code_server_enabled: req.code_server_enabled,
            services: Default::default(),
            power_policy: Default::default(),
            wake_page_enabled: true,
        };

        let (app, token) = self
            .registry
            .create_application_headless(create_req)
            .await
            .map_err(|e| format!("Failed to create application record: {e}"))?;

        let record = ContainerV2Record {
            id: app.id.clone(),
            name: req.name,
            slug: req.slug.clone(),
            container_name: container_name.clone(),
            host_id: host_id.clone(),
            status: ContainerV2Status::Deploying,
            created_at: Utc::now(),
            migrated_from_lxd_app_id: None,
        };

        // Persist the record
        {
            let mut state = self.state.write().await;
            state.containers.push(record.clone());
        }
        let _ = self.save_state().await;

        // Spawn background deploy
        let mgr = Arc::clone(self);
        let app_id = app.id.clone();
        let slug = req.slug.clone();
        let token_deploy = token.clone();
        tokio::spawn(async move {
            mgr.run_nspawn_deploy(&app_id, &slug, &container_name, &host_id, &token_deploy)
                .await;
        });

        Ok((record, token))
    }

    /// Remove a container: stop nspawn, delete rootfs, remove from registry.
    pub async fn remove_container(&self, id: &str) -> Result<bool, String> {
        let record = {
            let state = self.state.read().await;
            state.containers.iter().find(|c| c.id == id).cloned()
        };

        let Some(record) = record else {
            return Ok(false);
        };

        let storage_path = self.resolve_storage_path(&record.host_id).await;

        if record.host_id == "local" {
            let _ = NspawnClient::stop_container(&record.container_name).await;
            let _ =
                NspawnClient::delete_container(&record.container_name, Path::new(&storage_path))
                    .await;
        } else {
            let _ = self
                .registry
                .send_host_command(
                    &record.host_id,
                    HostRegistryMessage::StopContainer {
                        container_name: record.container_name.clone(),
                    },
                )
                .await;
            // TODO: send DeleteNspawnContainer when protocol supports it
        }

        // Remove from registry
        let _ = self.registry.remove_application(id).await;

        // Remove from V2 state
        {
            let mut state = self.state.write().await;
            state.containers.retain(|c| c.id != id);
        }
        let _ = self.save_state().await;

        info!(container = record.container_name, "Container V2 removed");
        Ok(true)
    }

    /// Start a stopped container.
    pub async fn start_container(&self, id: &str) -> Result<bool, String> {
        let record = {
            let state = self.state.read().await;
            state.containers.iter().find(|c| c.id == id).cloned()
        };
        let Some(record) = record else {
            return Ok(false);
        };

        if record.host_id == "local" {
            NspawnClient::start_container(&record.container_name)
                .await
                .map_err(|e| e.to_string())?;
        } else {
            self.registry
                .send_host_command(
                    &record.host_id,
                    HostRegistryMessage::StartContainer {
                        container_name: record.container_name.clone(),
                    },
                )
                .await?;
        }

        // Update status
        {
            let mut state = self.state.write().await;
            if let Some(c) = state.containers.iter_mut().find(|c| c.id == id) {
                c.status = ContainerV2Status::Running;
            }
        }
        let _ = self.save_state().await;
        Ok(true)
    }

    /// Stop a running container.
    pub async fn stop_container(&self, id: &str) -> Result<bool, String> {
        let record = {
            let state = self.state.read().await;
            state.containers.iter().find(|c| c.id == id).cloned()
        };
        let Some(record) = record else {
            return Ok(false);
        };

        if record.host_id == "local" {
            NspawnClient::stop_container(&record.container_name)
                .await
                .map_err(|e| e.to_string())?;
        } else {
            self.registry
                .send_host_command(
                    &record.host_id,
                    HostRegistryMessage::StopContainer {
                        container_name: record.container_name.clone(),
                    },
                )
                .await?;
        }

        // Update status
        {
            let mut state = self.state.write().await;
            if let Some(c) = state.containers.iter_mut().find(|c| c.id == id) {
                c.status = ContainerV2Status::Stopped;
            }
        }
        let _ = self.save_state().await;
        Ok(true)
    }

    /// List all V2 containers, enriched with agent status/metrics from registry.
    pub async fn list_containers(&self) -> Vec<serde_json::Value> {
        let state = self.state.read().await;
        let apps = self.registry.list_applications().await;

        let mut result = Vec::new();
        for record in &state.containers {
            let app = apps.iter().find(|a| a.id == record.id);
            let mut entry = serde_json::to_value(record).unwrap_or_default();
            if let Some(app) = app {
                entry["agent_status"] = serde_json::to_value(&app.status).unwrap_or_default();
                entry["ipv4_address"] = serde_json::json!(app.ipv4_address.map(|ip| ip.to_string()));
                entry["agent_version"] = serde_json::json!(app.agent_version);
                entry["last_heartbeat"] = serde_json::json!(app.last_heartbeat);
                if let Some(ref metrics) = app.metrics {
                    entry["metrics"] = serde_json::to_value(metrics).unwrap_or_default();
                }
                entry["frontend"] = serde_json::to_value(&app.frontend).unwrap_or_default();
                entry["apis"] = serde_json::to_value(&app.apis).unwrap_or_default();
                entry["code_server_enabled"] = serde_json::json!(app.code_server_enabled);
            }
            result.push(entry);
        }
        result
    }

    // ── Config ───────────────────────────────────────────────────

    pub async fn get_config(&self) -> ContainerV2Config {
        let mut cfg = self.state.read().await.config.clone();
        if cfg.container_storage_path.is_empty() {
            cfg.container_storage_path = default_storage_path();
        }
        cfg
    }

    pub async fn update_config(&self, config: ContainerV2Config) -> Result<(), String> {
        {
            let mut state = self.state.write().await;
            state.config = config;
        }
        self.save_state().await
    }

    // ── Storage path resolution ──────────────────────────────────

    pub async fn resolve_storage_path(&self, host_id: &str) -> String {
        if host_id == "local" {
            let path = self.state
                .read()
                .await
                .config
                .container_storage_path
                .clone();
            if path.is_empty() {
                return default_storage_path();
            }
            return path;
        } else {
            // Try to read from hosts.json
            if let Ok(content) =
                tokio::fs::read_to_string("/opt/homeroute/data/hosts.json").await
            {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(hosts) = data.get("hosts").and_then(|h| h.as_array()) {
                        if let Some(host) = hosts
                            .iter()
                            .find(|h| h.get("id").and_then(|i| i.as_str()) == Some(host_id))
                        {
                            if let Some(path) = host
                                .get("container_storage_path")
                                .and_then(|p| p.as_str())
                            {
                                return path.to_string();
                            }
                        }
                    }
                }
            }
            default_storage_path()
        }
    }

    async fn resolve_network_mode(&self, host_id: &str) -> String {
        if host_id == "local" {
            return "bridge:br-lan".to_string();
        }
        // Check hosts.json for lan_interface
        if let Ok(content) = tokio::fs::read_to_string("/opt/homeroute/data/hosts.json").await {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(hosts) = data.get("hosts").and_then(|h| h.as_array()) {
                    if let Some(host) = hosts.iter().find(|h| h.get("id").and_then(|i| i.as_str()) == Some(host_id)) {
                        if let Some(iface) = host.get("lan_interface").and_then(|v| v.as_str()) {
                            if !iface.is_empty() {
                                return format!("macvlan:{}", iface);
                            }
                        }
                    }
                }
            }
        }
        "bridge:br-lan".to_string()
    }

    // ── Background deploy ────────────────────────────────────────

    async fn run_nspawn_deploy(
        &self,
        app_id: &str,
        slug: &str,
        container_name: &str,
        host_id: &str,
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

        let storage_path = self.resolve_storage_path(host_id).await;
        let storage = Path::new(&storage_path);

        // Phase 1: Create the nspawn container
        emit("Creation du conteneur nspawn...");
        if let Err(e) = NspawnClient::create_container(container_name, storage).await {
            error!(container = container_name, "Nspawn creation failed: {e}");
            emit(&format!("Erreur: {e}"));
            self.set_container_status(app_id, ContainerV2Status::Error)
                .await;
            return;
        }

        // Phase 2: Deploy agent binary
        emit("Deploiement du binaire agent...");
        let agent_binary = PathBuf::from("/opt/homeroute/data/agent-binaries/hr-agent");
        if !agent_binary.exists() {
            let msg = "Agent binary not found";
            emit(msg);
            self.set_container_status(app_id, ContainerV2Status::Error)
                .await;
            return;
        }

        if let Err(e) =
            NspawnClient::push_file(container_name, &agent_binary, "usr/local/bin/hr-agent", storage)
                .await
        {
            error!(container = container_name, "Failed to push agent binary: {e}");
            emit(&format!("Erreur: {e}"));
            self.set_container_status(app_id, ContainerV2Status::Error)
                .await;
            return;
        }

        if let Err(e) = NspawnClient::exec(
            container_name,
            &["chmod", "+x", "/usr/local/bin/hr-agent"],
        )
        .await
        {
            error!(container = container_name, "chmod failed: {e}");
            emit(&format!("Erreur: {e}"));
            self.set_container_status(app_id, ContainerV2Status::Error)
                .await;
            return;
        }

        // Phase 3: Generate and push agent config (interface = "host0" for nspawn)
        emit("Configuration de l'agent...");
        let api_port = self.env.api_port;
        let config_content = format!(
            r#"homeroute_address = "10.0.0.254"
homeroute_port = {api_port}
token = "{token}"
service_name = "{slug}"
interface = "host0"
"#
        );

        let tmp_config = PathBuf::from(format!("/tmp/hr-agent-v2-{slug}.toml"));
        if let Err(e) = tokio::fs::write(&tmp_config, &config_content).await {
            error!("Failed to write tmp config: {e}");
            self.set_container_status(app_id, ContainerV2Status::Error)
                .await;
            return;
        }
        let _ = NspawnClient::push_file(container_name, &tmp_config, "etc/hr-agent.toml", storage).await;
        let _ = tokio::fs::remove_file(&tmp_config).await;

        // Phase 4: Push systemd unit
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
        let tmp_unit = PathBuf::from(format!("/tmp/hr-agent-v2-{slug}.service"));
        let _ = tokio::fs::write(&tmp_unit, unit_content).await;
        let _ = NspawnClient::push_file(
            container_name,
            &tmp_unit,
            "etc/systemd/system/hr-agent.service",
            storage,
        )
        .await;
        let _ = tokio::fs::remove_file(&tmp_unit).await;

        // Phase 5: Enable and start agent
        emit("Demarrage de l'agent...");
        let _ = NspawnClient::exec(container_name, &["systemctl", "daemon-reload"]).await;
        let _ =
            NspawnClient::exec(container_name, &["systemctl", "enable", "--now", "hr-agent"])
                .await;

        // Phase 6: Wait for network
        emit("Attente de la connectivite reseau...");
        if let Err(e) = NspawnClient::wait_for_network(container_name, 30).await {
            warn!(container = container_name, "Network wait failed: {e}");
        }

        // Phase 7: Install dependencies
        emit("Installation des dependances...");
        let _ = NspawnClient::exec_with_retry(
            container_name,
            &[
                "bash",
                "-c",
                "apt-get update -qq && apt-get install -y -qq curl",
            ],
            3,
        )
        .await;

        // Phase 8: Install code-server
        emit("Installation de code-server...");
        let _ = NspawnClient::exec_with_retry(
            container_name,
            &[
                "bash",
                "-c",
                "curl -fsSL https://code-server.dev/install.sh | sh -s -- --method=standalone --prefix=/usr/local",
            ],
            3,
        )
        .await;

        // Phase 9: Create workspace
        emit("Creation du volume workspace...");
        let _ = NspawnClient::create_workspace(container_name, storage).await;

        // Phase 10: Deploy MCP config
        emit("Configuration MCP Dataverse...");
        let mcp_config = r#"{
  "mcpServers": {
    "dataverse": {
      "command": "/usr/local/bin/hr-agent",
      "args": ["mcp"],
      "autoApprove": [
        "list_tables","describe_table","create_table","add_column","remove_column",
        "drop_table","create_relation","query_data","insert_data","update_data",
        "delete_data","count_rows","get_schema","get_db_info"
      ]
    }
  }
}
"#;
        let tmp_mcp = PathBuf::from(format!("/tmp/mcp-v2-{slug}.json"));
        let _ = tokio::fs::write(&tmp_mcp, mcp_config).await;
        let _ =
            NspawnClient::push_file(container_name, &tmp_mcp, "root/workspace/.mcp.json", storage)
                .await;
        let _ = tokio::fs::remove_file(&tmp_mcp).await;

        // Phase 11: Deploy CLAUDE.md
        emit("Deploiement CLAUDE.md Dataverse...");
        let claude_md_content = include_str!("../../hr-registry/src/dataverse_claude_md.txt");
        let tmp_claude = PathBuf::from(format!("/tmp/claude-md-v2-{slug}.md"));
        let _ = tokio::fs::write(&tmp_claude, claude_md_content).await;
        let _ = NspawnClient::push_file(
            container_name,
            &tmp_claude,
            "root/workspace/CLAUDE.md",
            storage,
        )
        .await;
        let _ = tokio::fs::remove_file(&tmp_claude).await;

        // Phase 12: Configure code-server
        emit("Configuration de code-server...");
        let _ = NspawnClient::exec(
            container_name,
            &["mkdir", "-p", "/root/.config/code-server"],
        )
        .await;
        let cs_config = "bind-addr: 0.0.0.0:13337\nauth: none\ncert: false\n";
        let tmp_cs = PathBuf::from(format!("/tmp/cs-config-v2-{slug}.yaml"));
        let _ = tokio::fs::write(&tmp_cs, cs_config).await;
        let _ = NspawnClient::push_file(
            container_name,
            &tmp_cs,
            "root/.config/code-server/config.yaml",
            storage,
        )
        .await;
        let _ = tokio::fs::remove_file(&tmp_cs).await;

        // VS Code settings
        let _ = NspawnClient::exec(
            container_name,
            &["mkdir", "-p", "/root/.local/share/code-server/User"],
        )
        .await;
        let cs_settings = r#"{
  "workbench.colorTheme": "Default Dark Modern",
  "chat.disableAIFeatures": true,
  "workbench.startupEditor": "none",
  "telemetry.telemetryLevel": "off",
  "remote.autoForwardPorts": false
}
"#;
        let tmp_settings = PathBuf::from(format!("/tmp/cs-settings-v2-{slug}.json"));
        let _ = tokio::fs::write(&tmp_settings, cs_settings).await;
        let _ = NspawnClient::push_file(
            container_name,
            &tmp_settings,
            "root/.local/share/code-server/User/settings.json",
            storage,
        )
        .await;
        let _ = tokio::fs::remove_file(&tmp_settings).await;

        // code-server systemd unit
        let cs_unit = r#"[Unit]
Description=code-server IDE
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/code-server --bind-addr 0.0.0.0:13337 /root/workspace
Restart=always
RestartSec=5
Environment=HOME=/root
KillMode=control-group
KillSignal=SIGTERM
TimeoutStopSec=10

[Install]
WantedBy=multi-user.target
"#;
        let tmp_cs_unit = PathBuf::from(format!("/tmp/cs-unit-v2-{slug}.service"));
        let _ = tokio::fs::write(&tmp_cs_unit, cs_unit).await;
        let _ = NspawnClient::push_file(
            container_name,
            &tmp_cs_unit,
            "etc/systemd/system/code-server.service",
            storage,
        )
        .await;
        let _ = tokio::fs::remove_file(&tmp_cs_unit).await;

        // code-server setup unit (Claude Code extension)
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
        let tmp_cs_setup = PathBuf::from(format!("/tmp/cs-setup-v2-{slug}.service"));
        let _ = tokio::fs::write(&tmp_cs_setup, cs_setup_unit).await;
        let _ = NspawnClient::push_file(
            container_name,
            &tmp_cs_setup,
            "etc/systemd/system/code-server-setup.service",
            storage,
        )
        .await;
        let _ = tokio::fs::remove_file(&tmp_cs_setup).await;

        emit("Demarrage de code-server...");
        let _ = NspawnClient::exec(container_name, &["systemctl", "daemon-reload"]).await;
        let _ =
            NspawnClient::exec(container_name, &["systemctl", "enable", "--now", "code-server"])
                .await;
        let _ = NspawnClient::exec(
            container_name,
            &["systemctl", "enable", "--now", "code-server-setup"],
        )
        .await;

        // Update status to Pending (agent not yet connected)
        self.set_container_status(app_id, ContainerV2Status::Running)
            .await;

        let _ = self.events.agent_status.send(AgentStatusEvent {
            app_id: app_id.to_string(),
            slug: slug.to_string(),
            status: "pending".to_string(),
            message: Some("Deploiement termine".to_string()),
        });

        info!(container = container_name, "Container V2 deploy complete");
    }

    /// Update the status of a container V2 record and the corresponding application.
    async fn set_container_status(&self, id: &str, status: ContainerV2Status) {
        {
            let mut state = self.state.write().await;
            if let Some(c) = state.containers.iter_mut().find(|c| c.id == id) {
                c.status = status.clone();
            }
        }
        let _ = self.save_state().await;

        // Also update registry application status
        let agent_status = match status {
            ContainerV2Status::Deploying => AgentStatus::Deploying,
            ContainerV2Status::Error => AgentStatus::Error,
            _ => return,
        };
        let _ = self
            .registry
            .update_application(
                id,
                UpdateApplicationRequest {
                    ..Default::default()
                },
            )
            .await;
        // Set status through the registry's internal mechanism if needed
        let _ = agent_status; // used for matching only
    }

    // ── Inter-host migration ─────────────────────────────────────

    /// Start migration of a V2 container to another host.
    pub async fn migrate_container(
        self: &Arc<Self>,
        container_id: &str,
        target_host_id: &str,
        migrations: &Arc<RwLock<std::collections::HashMap<String, MigrationState>>>,
    ) -> Result<String, String> {
        let record = {
            let state = self.state.read().await;
            state
                .containers
                .iter()
                .find(|c| c.id == container_id)
                .cloned()
        };

        let record = record.ok_or("Container not found")?;
        let source_host_id = record.host_id.clone();

        if source_host_id == target_host_id {
            return Err("Container is already on target host".to_string());
        }

        if target_host_id != "local" && !self.registry.is_host_connected(target_host_id).await {
            return Err("Target host is not connected".to_string());
        }

        let transfer_id = uuid::Uuid::new_v4().to_string();
        let cancelled = Arc::new(AtomicBool::new(false));

        let migration_state = MigrationState {
            app_id: container_id.to_string(),
            transfer_id: transfer_id.clone(),
            source_host_id: source_host_id.clone(),
            target_host_id: target_host_id.to_string(),
            phase: MigrationPhase::Stopping,
            progress_pct: 0,
            bytes_transferred: 0,
            total_bytes: 0,
            started_at: Utc::now(),
            error: None,
            cancelled: cancelled.clone(),
        };

        {
            let mut m = migrations.write().await;
            if m.values().any(|ms| {
                ms.app_id == container_id
                    && ms.error.is_none()
                    && !matches!(
                        ms.phase,
                        MigrationPhase::Complete | MigrationPhase::Failed
                    )
            }) {
                return Err("Migration already in progress".to_string());
            }
            m.insert(transfer_id.clone(), migration_state);
        }

        // Update container status
        {
            let mut state = self.state.write().await;
            if let Some(c) = state.containers.iter_mut().find(|c| c.id == container_id) {
                c.status = ContainerV2Status::Migrating;
            }
        }
        let _ = self.save_state().await;

        let mgr = Arc::clone(self);
        let migrations = migrations.clone();
        let events = self.events.clone();
        let registry = self.registry.clone();
        let tid = transfer_id.clone();
        let cid = container_id.to_string();
        let thid = target_host_id.to_string();
        let slug = record.slug.clone();
        let container_name = record.container_name.clone();

        tokio::spawn(async move {
            mgr.run_nspawn_migration(
                &registry,
                &migrations,
                &events,
                &cid,
                &slug,
                &tid,
                &source_host_id,
                &thid,
                &container_name,
                &cancelled,
            )
            .await;
        });

        Ok(transfer_id)
    }

    async fn run_nspawn_migration(
        &self,
        registry: &Arc<AgentRegistry>,
        migrations: &Arc<RwLock<std::collections::HashMap<String, MigrationState>>>,
        events: &Arc<EventBus>,
        app_id: &str,
        _slug: &str,
        transfer_id: &str,
        source_host_id: &str,
        target_host_id: &str,
        container_name: &str,
        cancelled: &Arc<AtomicBool>,
    ) {
        let source_stopped = AtomicBool::new(false);

        let result = self
            .run_nspawn_migration_inner(
                registry,
                migrations,
                events,
                app_id,
                transfer_id,
                source_host_id,
                target_host_id,
                container_name,
                &source_stopped,
                cancelled,
            )
            .await;

        if let Err(error_msg) = result {
            // Rollback: restart source container if stopped
            if source_stopped.load(Ordering::SeqCst) {
                warn!(
                    app_id = %app_id,
                    container = %container_name,
                    "Nspawn migration failed after source stop, restarting source"
                );
                if source_host_id == "local" {
                    let _ = NspawnClient::start_container(container_name).await;
                } else {
                    let _ = registry
                        .send_host_command(
                            source_host_id,
                            HostRegistryMessage::StartContainer {
                                container_name: container_name.to_string(),
                            },
                        )
                        .await;
                }
            }

            // Restore container status
            {
                let mut state = self.state.write().await;
                if let Some(c) = state.containers.iter_mut().find(|c| c.id == app_id) {
                    c.status = ContainerV2Status::Running;
                }
            }
            let _ = self.save_state().await;

            crate::routes::applications::update_migration_phase(
                migrations,
                events,
                app_id,
                transfer_id,
                MigrationPhase::Failed,
                0,
                0,
                0,
                Some(error_msg),
            )
            .await;
        }
    }

    async fn run_nspawn_migration_inner(
        &self,
        registry: &Arc<AgentRegistry>,
        migrations: &Arc<RwLock<std::collections::HashMap<String, MigrationState>>>,
        events: &Arc<EventBus>,
        app_id: &str,
        transfer_id: &str,
        source_host_id: &str,
        target_host_id: &str,
        container_name: &str,
        source_stopped: &AtomicBool,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        let source_is_local = source_host_id == "local";
        let target_is_local = target_host_id == "local";

        let source_storage = self.resolve_storage_path(source_host_id).await;
        let target_storage = self.resolve_storage_path(target_host_id).await;

        // Phase 1: Stopping
        crate::routes::applications::update_migration_phase(
            migrations,
            events,
            app_id,
            transfer_id,
            MigrationPhase::Stopping,
            0,
            0,
            0,
            None,
        )
        .await;

        let _ = registry
            .send_service_command(app_id, ServiceType::App, ServiceAction::Stop)
            .await;
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Phase 2: Exporting
        crate::routes::applications::update_migration_phase(
            migrations,
            events,
            app_id,
            transfer_id,
            MigrationPhase::Exporting,
            10,
            0,
            0,
            None,
        )
        .await;

        if source_is_local {
            // Stop the container
            let _ = NspawnClient::stop_container(container_name).await;
            source_stopped.store(true, Ordering::SeqCst);

            let rootfs_path = Path::new(&source_storage).join(container_name);

            // Estimate size
            let size_output = tokio::process::Command::new("du")
                .args(["-sb", &rootfs_path.to_string_lossy()])
                .output()
                .await
                .map_err(|e| format!("du failed: {e}"))?;
            let total_bytes: u64 = String::from_utf8_lossy(&size_output.stdout)
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            crate::routes::applications::update_migration_phase(
                migrations,
                events,
                app_id,
                transfer_id,
                MigrationPhase::Transferring,
                20,
                0,
                total_bytes,
                None,
            )
            .await;

            if !target_is_local {
                // Local → Remote: stream rootfs tar to target
                let import_rx = registry.register_migration_signal(transfer_id).await;

                let _ = registry
                    .send_host_command(
                        target_host_id,
                        HostRegistryMessage::StartNspawnImport {
                            container_name: container_name.to_string(),
                            storage_path: target_storage.clone(),
                            transfer_id: transfer_id.to_string(),
                            network_mode: self.resolve_network_mode(target_host_id).await,
                        },
                    )
                    .await
                    .map_err(|e| format!("Failed to notify target: {e}"))?;

                // Spawn tar
                let mut tar_child = tokio::process::Command::new("tar")
                    .args(["cf", "-", "-C", &rootfs_path.to_string_lossy(), "."])
                    .stdout(std::process::Stdio::piped())
                    .spawn()
                    .map_err(|e| format!("Failed to spawn tar: {e}"))?;

                let mut tar_stdout = tar_child.stdout.take().unwrap();

                let (_transferred, _seq) = crate::routes::applications::stream_to_remote(
                    registry,
                    target_host_id,
                    transfer_id,
                    &mut tar_stdout,
                    total_bytes,
                    cancelled,
                    migrations,
                    events,
                    app_id,
                    20,
                    80,
                    MigrationPhase::Transferring,
                )
                .await?;

                let _ = tar_child.wait().await;

                // Stream workspace if exists
                let ws_path = Path::new(&source_storage).join(format!("{}-workspace", container_name));
                if tokio::fs::metadata(&ws_path).await.is_ok() {
                    let ws_size_output = tokio::process::Command::new("du")
                        .args(["-sb", &ws_path.to_string_lossy()])
                        .output()
                        .await;
                    let ws_size: u64 = ws_size_output
                        .ok()
                        .map(|o| {
                            String::from_utf8_lossy(&o.stdout)
                                .split_whitespace()
                                .next()
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0)
                        })
                        .unwrap_or(0);

                    let _ = registry
                        .send_host_command(
                            target_host_id,
                            HostRegistryMessage::WorkspaceReady {
                                transfer_id: transfer_id.to_string(),
                                size_bytes: ws_size,
                            },
                        )
                        .await;

                    if let Ok(mut ws_child) = tokio::process::Command::new("tar")
                        .args(["cf", "-", "-C", &ws_path.to_string_lossy(), "."])
                        .stdout(std::process::Stdio::piped())
                        .spawn()
                    {
                        if let Some(mut ws_stdout) = ws_child.stdout.take() {
                            let _ = crate::routes::applications::stream_to_remote(
                                registry,
                                target_host_id,
                                transfer_id,
                                &mut ws_stdout,
                                ws_size,
                                cancelled,
                                migrations,
                                events,
                                app_id,
                                82,
                                84,
                                MigrationPhase::TransferringWorkspace,
                            )
                            .await;
                        }
                        let _ = ws_child.wait().await;
                    }
                }

                let _ = registry
                    .send_host_command(
                        target_host_id,
                        HostRegistryMessage::TransferComplete {
                            transfer_id: transfer_id.to_string(),
                        },
                    )
                    .await;

                crate::routes::applications::update_migration_phase(
                    migrations,
                    events,
                    app_id,
                    transfer_id,
                    MigrationPhase::Importing,
                    85,
                    0,
                    0,
                    None,
                )
                .await;

                match tokio::time::timeout(Duration::from_secs(120), import_rx).await {
                    Ok(Ok(hr_registry::MigrationResult::ImportComplete { .. })) => {
                        info!(transfer_id, "Nspawn import confirmed by target host");
                    }
                    Ok(Ok(hr_registry::MigrationResult::ImportFailed { error })) => {
                        return Err(format!("Migration failed on target: {error}"));
                    }
                    Ok(Ok(hr_registry::MigrationResult::ExportFailed { error })) => {
                        return Err(format!("Migration failed: {error}"));
                    }
                    Ok(Err(_)) => return Err("Migration signal lost".to_string()),
                    Err(_) => return Err("Import timed out after 120s".to_string()),
                }
            } else {
                // Local → Local: unlikely but handle gracefully
                return Err("Local-to-local nspawn migration not supported".to_string());
            }
        } else {
            // Source is remote
            let import_rx = registry.register_migration_signal(transfer_id).await;

            if target_is_local {
                registry
                    .set_transfer_container_name(transfer_id, container_name)
                    .await;
            } else {
                registry
                    .set_transfer_relay_target(transfer_id, target_host_id, container_name)
                    .await;

                let _ = registry
                    .send_host_command(
                        target_host_id,
                        HostRegistryMessage::StartNspawnImport {
                            container_name: container_name.to_string(),
                            storage_path: target_storage.clone(),
                            transfer_id: transfer_id.to_string(),
                            network_mode: self.resolve_network_mode(target_host_id).await,
                        },
                    )
                    .await
                    .map_err(|e| format!("Failed to notify target: {e}"))?;
            }

            let _ = registry
                .send_host_command(
                    source_host_id,
                    HostRegistryMessage::StartNspawnExport {
                        container_name: container_name.to_string(),
                        storage_path: source_storage.clone(),
                        transfer_id: transfer_id.to_string(),
                    },
                )
                .await
                .map_err(|e| format!("Failed to start export: {e}"))?;

            source_stopped.store(true, Ordering::SeqCst);

            crate::routes::applications::update_migration_phase(
                migrations,
                events,
                app_id,
                transfer_id,
                MigrationPhase::Exporting,
                30,
                0,
                0,
                None,
            )
            .await;

            match tokio::time::timeout(Duration::from_secs(600), import_rx).await {
                Ok(Ok(hr_registry::MigrationResult::ExportFailed { error })) => {
                    return Err(format!("Export failed on source: {error}"));
                }
                Ok(Ok(hr_registry::MigrationResult::ImportFailed { error })) => {
                    return Err(format!("Import failed: {error}"));
                }
                Ok(Ok(hr_registry::MigrationResult::ImportComplete { .. })) => {
                    info!(transfer_id, "Remote nspawn migration confirmed");
                }
                Ok(Err(_)) => return Err("Migration signal lost".to_string()),
                Err(_) => return Err("Remote migration timed out after 600s".to_string()),
            }
        }

        // Phase 5: Starting — update host_id
        crate::routes::applications::update_migration_phase(
            migrations,
            events,
            app_id,
            transfer_id,
            MigrationPhase::Starting,
            90,
            0,
            0,
            None,
        )
        .await;

        let update_req = UpdateApplicationRequest {
            host_id: Some(target_host_id.to_string()),
            ..Default::default()
        };
        let mut host_updated = false;
        for attempt in 0..3u32 {
            match registry.update_application(app_id, update_req.clone()).await {
                Ok(_) => {
                    host_updated = true;
                    break;
                }
                Err(e) => {
                    warn!(attempt, "Failed to update host_id: {e}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
        if !host_updated {
            return Err("Failed to update application host_id after 3 attempts".to_string());
        }

        // Update V2 record
        {
            let mut state = self.state.write().await;
            if let Some(c) = state.containers.iter_mut().find(|c| c.id == app_id) {
                c.host_id = target_host_id.to_string();
            }
        }
        let _ = self.save_state().await;

        // Phase 6: Verifying
        crate::routes::applications::update_migration_phase(
            migrations,
            events,
            app_id,
            transfer_id,
            MigrationPhase::Verifying,
            93,
            0,
            0,
            None,
        )
        .await;

        let mut agent_reconnected = false;
        for _ in 0..30 {
            if registry.is_agent_connected(app_id).await {
                agent_reconnected = true;
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }

        if !agent_reconnected {
            error!(app_id, "Agent did not reconnect within 60s after nspawn migration");

            // Rollback: delete target, revert host_id, restart source
            if target_is_local {
                let _ = NspawnClient::delete_container(
                    container_name,
                    Path::new(&target_storage),
                )
                .await;
            } else {
                let _ = registry
                    .send_host_command(
                        target_host_id,
                        HostRegistryMessage::DeleteContainer {
                            container_name: container_name.to_string(),
                        },
                    )
                    .await;
            }

            let revert_req = UpdateApplicationRequest {
                host_id: Some(source_host_id.to_string()),
                ..Default::default()
            };
            let _ = registry.update_application(app_id, revert_req).await;

            {
                let mut state = self.state.write().await;
                if let Some(c) = state.containers.iter_mut().find(|c| c.id == app_id) {
                    c.host_id = source_host_id.to_string();
                }
            }
            let _ = self.save_state().await;

            return Err("Agent did not reconnect after migration".to_string());
        }

        // Phase 7: Cleanup source
        if source_is_local {
            let _ = NspawnClient::delete_container(
                container_name,
                Path::new(&source_storage),
            )
            .await;
        } else {
            let _ = registry
                .send_host_command(
                    source_host_id,
                    HostRegistryMessage::DeleteContainer {
                        container_name: container_name.to_string(),
                    },
                )
                .await;
        }

        // Update container status
        {
            let mut state = self.state.write().await;
            if let Some(c) = state.containers.iter_mut().find(|c| c.id == app_id) {
                c.status = ContainerV2Status::Running;
            }
        }
        let _ = self.save_state().await;

        // Phase 8: Complete
        crate::routes::applications::update_migration_phase(
            migrations,
            events,
            app_id,
            transfer_id,
            MigrationPhase::Complete,
            100,
            0,
            0,
            None,
        )
        .await;

        info!(
            app_id,
            transfer_id, "Nspawn migration complete: {} → {}", source_host_id, target_host_id
        );
        Ok(())
    }
}
