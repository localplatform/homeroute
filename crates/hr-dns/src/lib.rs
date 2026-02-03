pub mod config;
pub mod records;
pub mod packet;
pub mod cache;
pub mod upstream;
pub mod resolver;
pub mod server;
pub mod logging;

pub use config::DnsConfig;

use std::collections::HashMap;
use std::net::Ipv6Addr;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use hr_common::events::DnsTrafficEvent;

/// Shared store for application DNS records (domain → IPv6).
/// Updated by the registry when agents connect/disconnect.
pub type AppDnsStore = Arc<RwLock<HashMap<String, Ipv6Addr>>>;

pub struct DnsState {
    pub config: config::DnsConfig,
    pub dns_cache: cache::DnsCache,
    pub upstream: upstream::UpstreamForwarder,
    pub query_logger: Option<logging::QueryLogger>,
    pub adblock: Arc<RwLock<hr_adblock::AdblockEngine>>,
    pub lease_store: Arc<RwLock<hr_dhcp::LeaseStore>>,
    pub adblock_enabled: bool,
    pub adblock_block_response: String,
    /// Event sender for DNS traffic analytics
    pub dns_events: Option<broadcast::Sender<DnsTrafficEvent>>,
    /// Application DNS records (domain → IPv6), updated by registry
    pub app_dns_store: AppDnsStore,
}

impl DnsState {
    pub fn server_ip(&self) -> std::net::Ipv4Addr {
        self.config.listen_addresses.first()
            .and_then(|s| s.parse().ok())
            .unwrap_or(std::net::Ipv4Addr::UNSPECIFIED)
    }
}

pub type SharedDnsState = Arc<RwLock<DnsState>>;
