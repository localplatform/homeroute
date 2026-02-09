use serde::{Deserialize, Serialize};

use crate::types::{ApiEndpoint, FrontendEndpoint};

// ── Shared Types ────────────────────────────────────────────────

/// State of a managed service (code-server, app, or db).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    /// Service is running normally.
    Running,
    /// Service is stopped (auto-stopped due to idle or never started).
    Stopped,
    /// Service is currently starting.
    Starting,
    /// Service is currently stopping.
    Stopping,
    /// Service was manually stopped by user (no auto-wake).
    ManuallyOff,
}

impl Default for ServiceState {
    fn default() -> Self {
        Self::Stopped
    }
}

/// Type of service being managed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceType {
    CodeServer,
    App,
    Db,
}

/// Action to perform on a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAction {
    Start,
    Stop,
}

/// Configuration of which systemd services to manage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// App service units (e.g., ["myapp.service"]).
    #[serde(default)]
    pub app: Vec<String>,
    /// Database service units (e.g., ["postgresql.service"]).
    #[serde(default)]
    pub db: Vec<String>,
}

/// Power-saving policy configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PowerPolicy {
    /// Idle timeout for code-server in seconds (None = never auto-stop).
    #[serde(default)]
    pub code_server_idle_timeout_secs: Option<u64>,
}

/// Metrics reported by the agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentMetrics {
    /// code-server service state.
    pub code_server_status: ServiceState,
    /// App services combined state.
    pub app_status: ServiceState,
    /// Database services combined state.
    pub db_status: ServiceState,
    /// RAM used in bytes.
    pub memory_bytes: u64,
    /// CPU usage percentage (0.0 - 100.0).
    pub cpu_percent: f32,
    /// Seconds since last code-server activity.
    pub code_server_idle_secs: u64,
}

// ── Messages from Agent → Registry ──────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgentMessage {
    /// Initial authentication when connecting.
    #[serde(rename = "auth")]
    Auth {
        token: String,
        service_name: String,
        version: String,
        /// Agent's IPv4 address (for local DNS A records).
        #[serde(default)]
        ipv4_address: Option<String>,
    },
    /// Periodic health report.
    #[serde(rename = "heartbeat")]
    Heartbeat {
        uptime_secs: u64,
        connections_active: u32,
    },
    /// Agent acknowledges a config push.
    #[serde(rename = "config_ack")]
    ConfigAck { config_version: u64 },
    /// Agent reports an error.
    #[serde(rename = "error")]
    Error { message: String },
    /// Agent reports system and service metrics.
    #[serde(rename = "metrics")]
    Metrics(AgentMetrics),
    /// Agent notifies that a service state changed.
    #[serde(rename = "service_state_changed")]
    ServiceStateChanged {
        service_type: ServiceType,
        new_state: ServiceState,
    },
    /// Agent publishes its routes for reverse proxy registration.
    #[serde(rename = "publish_routes")]
    PublishRoutes {
        routes: Vec<AgentRoute>,
    },
    /// Agent reports its Dataverse schema metadata.
    #[serde(rename = "schema_metadata")]
    SchemaMetadata {
        tables: Vec<SchemaTableInfo>,
        relations: Vec<SchemaRelationInfo>,
        version: u64,
        db_size_bytes: u64,
    },
}

/// A route published by an agent for reverse proxy registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRoute {
    pub domain: String,
    pub target_port: u16,
    pub service_type: ServiceType,
    pub auth_required: bool,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
}

/// Schema metadata reported by agent for Dataverse live view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaTableInfo {
    pub name: String,
    pub slug: String,
    pub columns: Vec<SchemaColumnInfo>,
    pub row_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaColumnInfo {
    pub name: String,
    pub field_type: String,
    pub required: bool,
    pub unique: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaRelationInfo {
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
    pub relation_type: String,
}

// ── Messages from Registry → Agent ──────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RegistryMessage {
    /// Response to Auth.
    #[serde(rename = "auth_result")]
    AuthResult {
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Full configuration push.
    #[serde(rename = "config")]
    Config {
        config_version: u64,
        /// Services to manage for powersave.
        #[serde(default)]
        services: ServiceConfig,
        /// Power-saving policy.
        #[serde(default)]
        power_policy: PowerPolicy,
        /// Base domain for route construction (e.g., "mynetwk.biz").
        #[serde(default)]
        base_domain: String,
        /// Application slug for route construction.
        #[serde(default)]
        slug: String,
        /// Frontend endpoint configuration.
        #[serde(default)]
        frontend: Option<FrontendEndpoint>,
        /// API endpoints.
        #[serde(default)]
        apis: Vec<ApiEndpoint>,
        /// Whether code-server is enabled.
        #[serde(default)]
        code_server_enabled: bool,
        /// Whether wake page is enabled for this app.
        #[serde(default = "default_true")]
        wake_page_enabled: bool,
    },
    /// Agent should self-update.
    #[serde(rename = "update_available")]
    UpdateAvailable {
        version: String,
        download_url: String,
        sha256: String,
    },
    /// Graceful shutdown request.
    #[serde(rename = "shutdown")]
    Shutdown,
    /// Update power policy (partial update).
    #[serde(rename = "power_policy_update")]
    PowerPolicyUpdate(PowerPolicy),
    /// Command to start/stop a specific service type.
    #[serde(rename = "service_command")]
    ServiceCommand {
        service_type: ServiceType,
        action: ServiceAction,
    },
    /// Activity ping to keep powersave timer alive.
    #[serde(rename = "activity_ping")]
    ActivityPing { service_type: ServiceType },
}

fn default_true() -> bool {
    true
}

/// Auto-off mode for idle host power management.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoOffMode {
    Sleep,
    Shutdown,
}

// ── Host Agent Protocol ──────────────────────────────────────────────────

/// Messages from host-agent → registry (via WebSocket)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum HostAgentMessage {
    Auth {
        token: String,
        host_name: String,
        version: String,
    },
    Heartbeat {
        uptime_secs: u64,
        containers_running: u32,
    },
    Metrics(HostMetrics),
    ContainerList(Vec<ContainerInfo>),
    ExportReady {
        transfer_id: String,
        #[serde(default)]
        container_name: String,
        size_bytes: u64,
    },
    /// Binary chunk announcement — the actual data follows as a WebSocket Binary frame.
    TransferChunkBinary {
        transfer_id: String,
        sequence: u32,
        size: u32,
        checksum: u32, // xxhash32
    },
    WorkspaceReady {
        transfer_id: String,
        size_bytes: u64,
    },
    TransferComplete {
        transfer_id: String,
    },
    ImportComplete {
        transfer_id: String,
        container_name: String,
    },
    ExportFailed {
        transfer_id: String,
        error: String,
    },
    ImportFailed {
        transfer_id: String,
        error: String,
    },
    ExecResult {
        request_id: String,
        success: bool,
        stdout: String,
        stderr: String,
    },
    NetworkInterfaces(Vec<NetworkInterfaceInfo>),
    /// Agent is about to auto-off (idle timeout reached).
    AutoOffNotify {
        mode: AutoOffMode,
    },
    /// Nspawn container list reported by host-agent.
    NspawnContainerList(Vec<NspawnContainerInfo>),
}

/// Nspawn container info reported by host-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NspawnContainerInfo {
    pub name: String,
    pub status: String,
    pub storage_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterfaceInfo {
    pub name: String,
    pub mac: String,
    pub ipv4: Option<String>,
    pub is_up: bool,
}

/// Host system metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostMetrics {
    pub cpu_percent: f32,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub load_avg: [f32; 3],
}

/// LXC container info reported by host-agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub name: String,
    pub status: String,
    pub ipv4: Option<String>,
}

/// Messages from registry → host-agent (via WebSocket)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum HostRegistryMessage {
    AuthResult {
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    CreateContainer {
        app_id: String,
        slug: String,
        config: String,
    },
    DeleteContainer {
        container_name: String,
    },
    StartContainer {
        container_name: String,
    },
    StopContainer {
        container_name: String,
    },
    PushAgentUpdate {
        version: String,
        download_url: String,
        sha256: String,
    },
    Shutdown {
        drain: bool,
    },
    /// Binary chunk announcement — the actual data follows as a WebSocket Binary frame.
    ReceiveChunkBinary {
        transfer_id: String,
        sequence: u32,
        size: u32,
        checksum: u32, // xxhash32
    },
    WorkspaceReady {
        transfer_id: String,
        size_bytes: u64,
    },
    TransferComplete {
        transfer_id: String,
    },
    ExecInContainer {
        request_id: String,
        container_name: String,
        command: Vec<String>,
    },
    PowerOff,
    Reboot,
    SuspendHost,
    SetAutoOff {
        mode: AutoOffMode,
        minutes: u32,
    },
    /// Cancel an in-flight migration transfer.
    CancelTransfer {
        transfer_id: String,
    },
    // ── Nspawn container management ──────────────────────────────
    CreateNspawnContainer {
        app_id: String,
        slug: String,
        container_name: String,
        storage_path: String,
        bridge: String,
        agent_token: String,
        agent_config: String,
    },
    DeleteNspawnContainer {
        container_name: String,
        storage_path: String,
    },
    StartNspawnContainer {
        container_name: String,
        storage_path: String,
    },
    StopNspawnContainer {
        container_name: String,
    },
    ExecInNspawnContainer {
        request_id: String,
        container_name: String,
        command: Vec<String>,
    },
    StartNspawnExport {
        container_name: String,
        storage_path: String,
        transfer_id: String,
    },
    StartNspawnImport {
        container_name: String,
        storage_path: String,
        transfer_id: String,
        network_mode: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_message_serde() {
        let msg = AgentMessage::Auth {
            token: "abc".into(),
            service_name: "test".into(),
            version: "0.1.0".into(),
            ipv4_address: Some("10.0.0.100".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"auth"#));
        let parsed: AgentMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            AgentMessage::Auth { token, .. } => assert_eq!(token, "abc"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_registry_message_serde() {
        let msg = RegistryMessage::AuthResult {
            success: true,
            error: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: RegistryMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            RegistryMessage::AuthResult { success, .. } => assert!(success),
            _ => panic!("wrong variant"),
        }
    }
}
