use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ipv6Config {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub ra_enabled: bool,
    #[serde(default)]
    pub ra_prefix: String,
    #[serde(default = "default_ra_lifetime")]
    pub ra_lifetime_secs: u32,
    #[serde(default)]
    pub ra_managed_flag: bool,
    #[serde(default)]
    pub ra_other_flag: bool,
    #[serde(default)]
    pub dhcpv6_enabled: bool,
    #[serde(default)]
    pub dhcpv6_dns_servers: Vec<String>,
    #[serde(default)]
    pub interface: String,
}

fn default_ra_lifetime() -> u32 { 1800 }

impl Default for Ipv6Config {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap()
    }
}
