use anyhow::{Context, Result};
use serde::Deserialize;

/// Agent configuration loaded from /etc/hr-agent.toml
#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    /// HomeRoute ULA address (e.g. "fd00:cafe::1")
    pub homeroute_address: String,
    /// HomeRoute API port (e.g. 3017)
    pub homeroute_port: u16,
    /// Agent authentication token (64-char hex)
    pub token: String,
    /// Service/application name (slug)
    pub service_name: String,
    /// Network interface for IPv6 address assignment
    #[serde(default = "default_interface")]
    pub interface: String,
}

fn default_interface() -> String {
    "eth0".to_string()
}

impl AgentConfig {
    pub fn load(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config from {path}"))?;
        toml::from_str(&content)
            .with_context(|| format!("Failed to parse TOML config from {path}"))
    }

    /// WebSocket URL to connect to HomeRoute registry
    pub fn ws_url(&self) -> String {
        // IPv6 addresses need brackets, IPv4 addresses don't
        let host = if self.homeroute_address.contains(':') {
            format!("[{}]", self.homeroute_address)
        } else {
            self.homeroute_address.clone()
        };
        format!(
            "ws://{}:{}/api/applications/agents/ws",
            host, self.homeroute_port
        )
    }
}
