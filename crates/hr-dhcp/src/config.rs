use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhcpConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub interface: String,
    #[serde(default)]
    pub range_start: String,
    #[serde(default)]
    pub range_end: String,
    #[serde(default = "default_netmask")]
    pub netmask: String,
    #[serde(default)]
    pub gateway: String,
    #[serde(default)]
    pub dns_server: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default = "default_lease_time")]
    pub default_lease_time_secs: u64,
    #[serde(default)]
    pub authoritative: bool,
    #[serde(default = "default_lease_file")]
    pub lease_file: String,
    #[serde(default)]
    pub static_leases: Vec<StaticLease>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticLease {
    pub mac: String,
    pub ip: String,
    #[serde(default)]
    pub hostname: String,
}

fn default_true() -> bool {
    true
}

fn default_netmask() -> String {
    "255.255.255.0".to_string()
}

fn default_lease_time() -> u64 {
    86400
}

fn default_lease_file() -> String {
    "/var/lib/server-dashboard/dhcp-leases".to_string()
}

impl Default for DhcpConfig {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DhcpConfig::default();
        assert!(config.enabled);
        assert_eq!(config.netmask, "255.255.255.0");
        assert_eq!(config.default_lease_time_secs, 86400);
        assert_eq!(config.lease_file, "/var/lib/server-dashboard/dhcp-leases");
    }

    #[test]
    fn test_deserialize() {
        let json = r#"{
            "enabled": true,
            "range_start": "10.0.0.10",
            "range_end": "10.0.0.200",
            "gateway": "10.0.0.1",
            "dns_server": "10.0.0.1"
        }"#;
        let config: DhcpConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.range_start, "10.0.0.10");
        assert_eq!(config.range_end, "10.0.0.200");
    }
}
