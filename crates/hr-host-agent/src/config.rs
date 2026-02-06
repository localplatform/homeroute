use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub homeroute_url: String,
    pub token: String,
    pub host_name: String,
    #[serde(default = "default_reconnect")]
    pub reconnect_interval_secs: u64,
    /// Physical LAN interface for macvlan (e.g., "enp7s0f0"). Required for container migrations.
    #[serde(default)]
    pub lan_interface: Option<String>,
}

fn default_reconnect() -> u64 {
    5
}

impl Config {
    pub fn load(path: &PathBuf) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config {}: {}", path.display(), e))?;
        toml::from_str(&content)
            .map_err(|e| format!("Failed to parse config: {}", e))
    }

    pub fn ws_url(&self) -> String {
        format!("ws://{}/api/hosts/agent/ws", self.homeroute_url)
    }
}
