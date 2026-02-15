use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

use crate::protocol::{AgentMetrics, PowerPolicy, ServiceConfig, ServiceType};

/// Port that code-server listens on inside each container.
pub const CODE_SERVER_PORT: u16 = 13337;

/// Application environment: development or production.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Development,
    Production,
}

impl Default for Environment {
    fn default() -> Self {
        Self::Development
    }
}

/// A registered application with its container and agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub slug: String,
    /// Host this application belongs to ("local" for the main server).
    #[serde(default = "default_host_id")]
    pub host_id: String,
    /// Environment: development or production.
    #[serde(default)]
    pub environment: Environment,
    /// Linked app ID (dev ↔ prod pairing).
    #[serde(default)]
    pub linked_app_id: Option<String>,
    pub enabled: bool,
    pub container_name: String,
    /// Argon2 hash of the agent token.
    pub token_hash: String,
    /// IPv4 address reported by agent (for local DNS A records).
    #[serde(default)]
    pub ipv4_address: Option<Ipv4Addr>,
    pub status: AgentStatus,
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub agent_version: Option<String>,
    pub created_at: DateTime<Utc>,

    /// Frontend endpoint configuration.
    pub frontend: FrontendEndpoint,

    /// Whether code-server IDE is enabled for this application.
    #[serde(default = "default_true")]
    pub code_server_enabled: bool,

    /// Systemd services to manage for powersave.
    #[serde(default)]
    pub services: ServiceConfig,
    /// Power-saving policy.
    #[serde(default)]
    pub power_policy: PowerPolicy,
    /// Whether to show a wake page when service is starting (vs transparent wait).
    #[serde(default = "default_true")]
    pub wake_page_enabled: bool,
    /// Current metrics from agent (volatile, not persisted to disk).
    #[serde(skip_deserializing)]
    pub metrics: Option<AgentMetrics>,
}

impl Application {
    /// Return all domains this application serves.
    /// Dev: `code.{slug}.{base}` (if code_server_enabled).
    /// Prod: `{slug}.{base}`.
    pub fn domains(&self, base_domain: &str) -> Vec<String> {
        match self.environment {
            Environment::Development => {
                let mut domains = vec![];
                if self.code_server_enabled {
                    domains.push(format!("code.{}.{}", self.slug, base_domain));
                }
                domains
            }
            Environment::Production => {
                vec![format!("{}.{}", self.slug, base_domain)]
            }
        }
    }

    /// Return all (domain, port, auth_required, allowed_groups) tuples for agent routing.
    /// Dev: `code.{slug}.{base}` (if code_server_enabled).
    /// Prod: `{slug}.{base}`.
    pub fn routes(&self, base_domain: &str) -> Vec<RouteInfo> {
        match self.environment {
            Environment::Development => {
                let mut routes = vec![];
                if self.code_server_enabled {
                    routes.push(RouteInfo {
                        domain: format!("code.{}.{}", self.slug, base_domain),
                        target_port: CODE_SERVER_PORT,
                        auth_required: true,
                        allowed_groups: vec![],
                        service_type: ServiceType::CodeServer,
                    });
                }
                routes
            }
            Environment::Production => {
                vec![RouteInfo {
                    domain: format!("{}.{}", self.slug, base_domain),
                    target_port: self.frontend.target_port,
                    auth_required: self.frontend.auth_required,
                    allowed_groups: self.frontend.allowed_groups.clone(),
                    service_type: ServiceType::App,
                }]
            }
        }
    }

    /// Return the wildcard domain for this application's per-app certificate.
    /// e.g., `*.{slug}.{base_domain}`
    pub fn wildcard_domain(&self, base_domain: &str) -> String {
        format!("*.{}.{}", self.slug, base_domain)
    }
}

/// Route metadata for proxy registration at startup and agent-driven publishing.
#[derive(Debug, Clone)]
pub struct RouteInfo {
    pub domain: String,
    pub target_port: u16,
    pub auth_required: bool,
    pub allowed_groups: Vec<String>,
    pub service_type: ServiceType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendEndpoint {
    pub target_port: u16,
    #[serde(default)]
    pub auth_required: bool,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub local_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Pending,
    Deploying,
    Connected,
    Disconnected,
    Error,
}

/// Persisted registry state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryState {
    #[serde(default)]
    pub applications: Vec<Application>,
}

fn default_true() -> bool {
    true
}

fn default_host_id() -> String {
    "local".to_string()
}

impl Default for RegistryState {
    fn default() -> Self {
        Self {
            applications: Vec::new(),
        }
    }
}

/// Request body for creating an application via the API.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateApplicationRequest {
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub host_id: Option<String>,
    pub frontend: FrontendEndpoint,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub linked_app_id: Option<String>,
    #[serde(default = "default_true")]
    pub code_server_enabled: bool,
    #[serde(default)]
    pub services: ServiceConfig,
    #[serde(default)]
    pub power_policy: PowerPolicy,
    #[serde(default = "default_true")]
    pub wake_page_enabled: bool,
}

/// Request body for updating an application.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct UpdateApplicationRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub host_id: Option<String>,
    #[serde(default)]
    pub frontend: Option<FrontendEndpoint>,
    #[serde(default)]
    pub linked_app_id: Option<String>,
    #[serde(default)]
    pub code_server_enabled: Option<bool>,
    #[serde(default)]
    pub services: Option<ServiceConfig>,
    #[serde(default)]
    pub power_policy: Option<PowerPolicy>,
    #[serde(default)]
    pub wake_page_enabled: Option<bool>,
}

// ── Agent Update Types ──────────────────────────────────────────

/// Request body for triggering agent updates.
#[derive(Debug, Clone, Deserialize)]
pub struct TriggerUpdateRequest {
    /// Specific agent IDs to update (None = all connected agents).
    #[serde(default)]
    pub agent_ids: Option<Vec<String>>,
}

/// Result of notifying a single agent about an update.
#[derive(Debug, Clone, Serialize)]
pub struct AgentNotifyResult {
    pub id: String,
    pub slug: String,
    pub status: String,
}

/// Result of skipping a single agent.
#[derive(Debug, Clone, Serialize)]
pub struct AgentSkipResult {
    pub id: String,
    pub slug: String,
    pub reason: String,
}

/// Result of triggering updates to a batch of agents.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateBatchResult {
    pub version: String,
    pub sha256: String,
    pub agents_notified: Vec<AgentNotifyResult>,
    pub agents_skipped: Vec<AgentSkipResult>,
}

/// Update status for a single agent.
#[derive(Debug, Clone, Serialize)]
pub struct AgentUpdateStatusInfo {
    pub id: String,
    pub slug: String,
    pub container_name: String,
    pub status: String,
    pub current_version: Option<String>,
    pub update_status: String,
    pub metrics_flowing: bool,
    pub last_heartbeat: Option<DateTime<Utc>>,
}

/// Result of checking update status for all agents.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateStatusResult {
    pub expected_version: String,
    pub agents: Vec<AgentUpdateStatusInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_app(environment: Environment, code_server_enabled: bool) -> Application {
        Application {
            id: "test".into(),
            name: "Test".into(),
            slug: "myapp".into(),
            host_id: "local".into(),
            environment,
            linked_app_id: None,
            enabled: true,
            container_name: "hr-myapp".into(),
            token_hash: String::new(),
            ipv4_address: None,
            status: AgentStatus::Pending,
            last_heartbeat: None,
            agent_version: None,
            created_at: Utc::now(),
            frontend: FrontendEndpoint {
                target_port: 3000,
                auth_required: false,
                allowed_groups: vec![],
                local_only: false,
            },
            code_server_enabled,
            services: ServiceConfig::default(),
            power_policy: PowerPolicy::default(),
            wake_page_enabled: true,
            metrics: None,
        }
    }

    #[test]
    fn test_domains() {
        // Dev with code-server
        let app = make_test_app(Environment::Development, true);
        let domains = app.domains("example.com");
        assert_eq!(domains, vec!["code.myapp.example.com"]);

        // Prod
        let app = make_test_app(Environment::Production, true);
        let domains = app.domains("example.com");
        assert_eq!(domains, vec!["myapp.example.com"]);
    }

    #[test]
    fn test_domains_no_code_server() {
        let app = make_test_app(Environment::Development, false);
        let domains = app.domains("example.com");
        assert!(domains.is_empty());
    }

    #[test]
    fn test_routes_code_server() {
        let app = make_test_app(Environment::Development, true);
        let routes = app.routes("example.com");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].domain, "code.myapp.example.com");
        assert_eq!(routes[0].target_port, CODE_SERVER_PORT);
        assert!(routes[0].auth_required);
    }

    #[test]
    fn test_routes_production() {
        let app = make_test_app(Environment::Production, true);
        let routes = app.routes("example.com");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].domain, "myapp.example.com");
    }

    #[test]
    fn test_wildcard_domain() {
        let app = make_test_app(Environment::Development, true);
        assert_eq!(app.wildcard_domain("example.com"), "*.myapp.example.com");
    }

    #[test]
    fn test_serde_roundtrip() {
        let state = RegistryState::default();
        let json = serde_json::to_string(&state).unwrap();
        let parsed: RegistryState = serde_json::from_str(&json).unwrap();
        assert!(parsed.applications.is_empty());
    }
}
