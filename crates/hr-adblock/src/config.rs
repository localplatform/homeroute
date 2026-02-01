use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdblockConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_block_response")]
    pub block_response: String,
    #[serde(default = "default_api_port")]
    pub api_port: u16,
    #[serde(default)]
    pub sources: Vec<AdblockSource>,
    #[serde(default)]
    pub whitelist: Vec<String>,
    #[serde(default = "default_adblock_data_dir")]
    pub data_dir: String,
    #[serde(default = "default_auto_update_hours")]
    pub auto_update_hours: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdblockSource {
    pub name: String,
    pub url: String,
    #[serde(default = "default_source_format")]
    pub format: String,
}

fn default_true() -> bool {
    true
}
fn default_block_response() -> String {
    "zero_ip".to_string()
}
fn default_api_port() -> u16 {
    5380
}
fn default_adblock_data_dir() -> String {
    "/var/lib/server-dashboard/adblock".to_string()
}
fn default_auto_update_hours() -> u64 {
    24
}
fn default_source_format() -> String {
    "hosts".to_string()
}

impl Default for AdblockConfig {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap()
    }
}

impl AdblockConfig {
    pub fn load_from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn save_to_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        let tmp_path = path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &content)?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AdblockConfig::default();
        assert!(config.enabled);
        assert_eq!(config.api_port, 5380);
        assert_eq!(config.block_response, "zero_ip");
    }
}
