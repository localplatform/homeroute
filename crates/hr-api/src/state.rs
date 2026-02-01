use hr_adblock::AdblockEngine;
use hr_analytics::store::AnalyticsStore;
use hr_auth::AuthService;
use hr_ca::CertificateAuthority;
use hr_common::config::EnvConfig;
use hr_common::events::EventBus;
use hr_dns::SharedDnsState;
use hr_dhcp::SharedDhcpState;
use hr_proxy::ProxyState;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared application state for all API routes.
#[derive(Clone)]
pub struct ApiState {
    pub auth: Arc<AuthService>,
    pub ca: Arc<CertificateAuthority>,
    pub proxy: Arc<ProxyState>,
    pub dns: SharedDnsState,
    pub dhcp: SharedDhcpState,
    pub adblock: Arc<RwLock<AdblockEngine>>,
    pub events: Arc<EventBus>,
    pub env: Arc<EnvConfig>,
    pub analytics: Arc<AnalyticsStore>,

    /// Path to dns-dhcp-config.json
    pub dns_dhcp_config_path: PathBuf,
    /// Path to rust-proxy-config.json
    pub proxy_config_path: PathBuf,
    /// Path to reverseproxy-config.json
    pub reverseproxy_config_path: PathBuf,
}
