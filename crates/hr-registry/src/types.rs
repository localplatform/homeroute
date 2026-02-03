use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::Ipv6Addr;

use crate::protocol::{AgentMetrics, PowerPolicy, ServiceConfig};

/// Port that code-server listens on inside each container.
pub const CODE_SERVER_PORT: u16 = 13337;

/// A registered application with its LXC container and agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub enabled: bool,
    pub container_name: String,
    /// Argon2 hash of the agent token.
    pub token_hash: String,
    /// Stable host suffix for IPv6 address (combined with PD prefix).
    pub ipv6_suffix: u16,
    /// Currently assigned GUA (None if no prefix available).
    pub ipv6_address: Option<Ipv6Addr>,
    pub status: AgentStatus,
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub agent_version: Option<String>,
    pub created_at: DateTime<Utc>,

    /// Frontend endpoint configuration.
    pub frontend: FrontendEndpoint,
    /// Optional API endpoints (each gets a sub-domain).
    #[serde(default)]
    pub apis: Vec<ApiEndpoint>,

    /// Whether code-server IDE is enabled for this application.
    #[serde(default = "default_true")]
    pub code_server_enabled: bool,

    /// Systemd services to manage for powersave.
    #[serde(default)]
    pub services: ServiceConfig,
    /// Power-saving policy.
    #[serde(default)]
    pub power_policy: PowerPolicy,
    /// Current metrics from agent (volatile, not persisted to disk).
    #[serde(skip)]
    pub metrics: Option<AgentMetrics>,

    /// Certificate IDs (one per domain: frontend + each API).
    #[serde(default)]
    pub cert_ids: Vec<String>,
    /// Cloudflare DNS record IDs (one per domain).
    #[serde(default)]
    pub cloudflare_record_ids: Vec<String>,
}

impl Application {
    /// Return all domains this application serves.
    pub fn domains(&self, base_domain: &str) -> Vec<String> {
        let mut domains = vec![format!("{}.{}", self.slug, base_domain)];
        for api in &self.apis {
            domains.push(format!("{}-{}.{}", self.slug, api.slug, base_domain));
        }
        if self.code_server_enabled {
            domains.push(format!("{}.code.{}", self.slug, base_domain));
        }
        domains
    }

    /// Return all (domain, port, auth_required, allowed_groups) tuples for agent routing.
    pub fn routes(&self, base_domain: &str) -> Vec<RouteInfo> {
        let mut routes = vec![RouteInfo {
            domain: format!("{}.{}", self.slug, base_domain),
            target_port: self.frontend.target_port,
            auth_required: self.frontend.auth_required,
            allowed_groups: self.frontend.allowed_groups.clone(),
        }];
        for api in &self.apis {
            routes.push(RouteInfo {
                domain: format!("{}-{}.{}", self.slug, api.slug, base_domain),
                target_port: api.target_port,
                auth_required: api.auth_required,
                allowed_groups: api.allowed_groups.clone(),
            });
        }
        if self.code_server_enabled {
            routes.push(RouteInfo {
                domain: format!("{}.code.{}", self.slug, base_domain),
                target_port: CODE_SERVER_PORT,
                auth_required: true,
                allowed_groups: vec![],
            });
        }
        routes
    }
}

/// Temporary helper for route iteration.
#[derive(Debug, Clone)]
pub struct RouteInfo {
    pub domain: String,
    pub target_port: u16,
    pub auth_required: bool,
    pub allowed_groups: Vec<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEndpoint {
    pub slug: String,
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
    #[serde(default = "default_next_suffix")]
    pub next_suffix: u16,
}

fn default_true() -> bool {
    true
}

fn default_next_suffix() -> u16 {
    2 // Start at ::2 (::1 is HomeRoute itself)
}

impl Default for RegistryState {
    fn default() -> Self {
        Self {
            applications: Vec::new(),
            next_suffix: default_next_suffix(),
        }
    }
}

/// Request body for creating an application via the API.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateApplicationRequest {
    pub name: String,
    pub slug: String,
    pub frontend: FrontendEndpoint,
    #[serde(default)]
    pub apis: Vec<ApiEndpoint>,
    #[serde(default = "default_true")]
    pub code_server_enabled: bool,
    #[serde(default)]
    pub services: ServiceConfig,
    #[serde(default)]
    pub power_policy: PowerPolicy,
}

/// Request body for updating an application.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateApplicationRequest {
    pub name: Option<String>,
    pub frontend: Option<FrontendEndpoint>,
    pub apis: Option<Vec<ApiEndpoint>>,
    pub code_server_enabled: Option<bool>,
    pub services: Option<ServiceConfig>,
    pub power_policy: Option<PowerPolicy>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_app(code_server_enabled: bool) -> Application {
        Application {
            id: "test".into(),
            name: "Test".into(),
            slug: "myapp".into(),
            enabled: true,
            container_name: "hr-myapp".into(),
            token_hash: String::new(),
            ipv6_suffix: 2,
            ipv6_address: None,
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
            apis: vec![ApiEndpoint {
                slug: "api".into(),
                target_port: 3001,
                auth_required: true,
                allowed_groups: vec!["admin".into()],
                local_only: false,
            }],
            code_server_enabled,
            services: ServiceConfig::default(),
            power_policy: PowerPolicy::default(),
            metrics: None,
            cert_ids: vec![],
            cloudflare_record_ids: vec![],
        }
    }

    #[test]
    fn test_domains() {
        let app = make_test_app(true);
        let domains = app.domains("example.com");
        assert_eq!(
            domains,
            vec![
                "myapp.example.com",
                "myapp-api.example.com",
                "myapp.code.example.com",
            ]
        );
    }

    #[test]
    fn test_domains_no_code_server() {
        let app = make_test_app(false);
        let domains = app.domains("example.com");
        assert_eq!(domains, vec!["myapp.example.com", "myapp-api.example.com"]);
    }

    #[test]
    fn test_routes_code_server() {
        let app = make_test_app(true);
        let routes = app.routes("example.com");
        assert_eq!(routes.len(), 3);
        let cs_route = &routes[2];
        assert_eq!(cs_route.domain, "myapp.code.example.com");
        assert_eq!(cs_route.target_port, CODE_SERVER_PORT);
        assert!(cs_route.auth_required);
    }

    #[test]
    fn test_serde_roundtrip() {
        let state = RegistryState::default();
        let json = serde_json::to_string(&state).unwrap();
        let parsed: RegistryState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.next_suffix, 2);
        assert!(parsed.applications.is_empty());
    }
}
