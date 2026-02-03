use serde::{Deserialize, Serialize};

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
    /// Idle timeout for app/db services in seconds (None = never auto-stop).
    #[serde(default)]
    pub app_idle_timeout_secs: Option<u64>,
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
    /// Seconds since last app/db activity.
    pub app_idle_secs: u64,
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
        /// Agent's actual GUA IPv6 address (obtained via DHCPv6/SLAAC).
        #[serde(default)]
        ipv6_address: Option<String>,
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
        ipv6_address: String,
        routes: Vec<AgentRoute>,
        ca_pem: String,
        homeroute_auth_url: String,
        /// Public HTTPS dashboard URL for loading pages.
        #[serde(default)]
        dashboard_url: String,
        /// Services to manage for powersave.
        #[serde(default)]
        services: ServiceConfig,
        /// Power-saving policy.
        #[serde(default)]
        power_policy: PowerPolicy,
    },
    /// Partial update: IPv6 changed (prefix rotation).
    #[serde(rename = "ipv6_update")]
    Ipv6Update { ipv6_address: String },
    /// Partial update: certificates renewed.
    #[serde(rename = "cert_update")]
    CertUpdate { routes: Vec<AgentRoute> },
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
}

/// A single route the agent must serve (one per domain).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRoute {
    pub domain: String,
    pub target_port: u16,
    pub cert_pem: String,
    pub key_pem: String,
    pub auth_required: bool,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
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
            ipv6_address: Some("2a0d:3341:b5b1:7500::18".into()),
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
