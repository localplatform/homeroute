use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsConfig {
    #[serde(default = "default_listen_addresses")]
    pub listen_addresses: Vec<String>,
    #[serde(default = "default_dns_port")]
    pub port: u16,
    #[serde(default = "default_upstream_servers")]
    pub upstream_servers: Vec<String>,
    #[serde(default = "default_upstream_timeout")]
    pub upstream_timeout_ms: u64,
    #[serde(default = "default_cache_size")]
    pub cache_size: usize,
    #[serde(default)]
    pub local_domain: String,
    #[serde(default)]
    pub wildcard_ipv4: String,
    #[serde(default)]
    pub wildcard_ipv6: String,
    #[serde(default)]
    pub static_records: Vec<StaticRecord>,
    #[serde(default = "default_true")]
    pub expand_hosts: bool,
    #[serde(default)]
    pub query_log_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticRecord {
    pub name: String,
    #[serde(rename = "type")]
    pub record_type: String,
    pub value: String,
    #[serde(default = "default_ttl")]
    pub ttl: u32,
}

/// Adblock resolver config: the subset of adblock config that the DNS resolver needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdblockResolverConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_block_response")]
    pub block_response: String,
}

// Default functions
fn default_listen_addresses() -> Vec<String> {
    vec!["0.0.0.0".to_string()]
}
fn default_dns_port() -> u16 {
    53
}
fn default_upstream_servers() -> Vec<String> {
    vec!["1.1.1.1".to_string(), "8.8.8.8".to_string()]
}
fn default_upstream_timeout() -> u64 {
    3000
}
fn default_cache_size() -> usize {
    1000
}
fn default_ttl() -> u32 {
    300
}
fn default_true() -> bool {
    true
}
fn default_block_response() -> String {
    "zero_ip".to_string()
}

impl Default for DnsConfig {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap()
    }
}

impl Default for AdblockResolverConfig {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_dns_config() {
        let config = DnsConfig::default();
        assert_eq!(config.port, 53);
        assert_eq!(config.cache_size, 1000);
        assert!(config.expand_hosts);
        assert_eq!(config.upstream_servers.len(), 2);
    }

    #[test]
    fn test_roundtrip() {
        let json = r#"{
            "port": 5353,
            "local_domain": "test.lab"
        }"#;
        let config: DnsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.port, 5353);
        assert_eq!(config.local_domain, "test.lab");

        let serialized = serde_json::to_string(&config).unwrap();
        let config2: DnsConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(config2.port, 5353);
    }

    #[test]
    fn test_adblock_resolver_config_defaults() {
        let config = AdblockResolverConfig::default();
        assert!(config.enabled);
        assert_eq!(config.block_response, "zero_ip");
    }
}
