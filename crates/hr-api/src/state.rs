use hr_adblock::AdblockEngine;
use hr_auth::AuthService;
use hr_acme::AcmeManager;
use hr_common::config::EnvConfig;
use hr_common::events::{CloudRelayStatus, EventBus, MigrationPhase};
use hr_common::service_registry::SharedServiceRegistry;
use hr_dns::SharedDnsState;
use hr_dhcp::SharedDhcpState;

use hr_proxy::{ProxyState, TlsManager};
use hr_registry::AgentRegistry;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory state of an active migration.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MigrationState {
    pub app_id: String,
    pub transfer_id: String,
    pub source_host_id: String,
    pub target_host_id: String,
    pub phase: MigrationPhase,
    pub progress_pct: u8,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub error: Option<String>,
}

/// Cached Dataverse schema metadata for an application.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CachedDataverseSchema {
    pub app_id: String,
    pub slug: String,
    pub tables: Vec<CachedTableInfo>,
    pub relations: Vec<CachedRelationInfo>,
    pub version: u64,
    pub db_size_bytes: u64,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CachedTableInfo {
    pub name: String,
    pub slug: String,
    pub columns: Vec<CachedColumnInfo>,
    pub row_count: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CachedColumnInfo {
    pub name: String,
    pub field_type: String,
    pub required: bool,
    pub unique: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CachedRelationInfo {
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
    pub relation_type: String,
}

/// Live cloud relay connection info (updated by tunnel client).
pub struct CloudRelayInfo {
    pub status: CloudRelayStatus,
    pub vps_ipv4: Option<String>,
    pub latency_ms: Option<u64>,
    pub active_streams: Option<u32>,
}

/// Shared application state for all API routes.
#[derive(Clone)]
pub struct ApiState {
    pub auth: Arc<AuthService>,
    pub acme: Arc<AcmeManager>,
    pub proxy: Arc<ProxyState>,
    pub tls_manager: Arc<TlsManager>,
    pub dns: SharedDnsState,
    pub dhcp: SharedDhcpState,
    pub adblock: Arc<RwLock<AdblockEngine>>,
    pub events: Arc<EventBus>,
    pub env: Arc<EnvConfig>,
    pub service_registry: SharedServiceRegistry,

    pub registry: Option<Arc<AgentRegistry>>,

    /// Active migrations keyed by transfer_id.
    pub migrations: Arc<RwLock<HashMap<String, MigrationState>>>,

    /// Cached Dataverse schemas keyed by app_id.
    pub dataverse_schemas: Arc<RwLock<HashMap<String, CachedDataverseSchema>>>,

    /// Live cloud relay connection status.
    pub cloud_relay_status: Arc<RwLock<Option<CloudRelayInfo>>>,

    /// Path to dns-dhcp-config.json
    pub dns_dhcp_config_path: PathBuf,
    /// Path to rust-proxy-config.json
    pub proxy_config_path: PathBuf,
    /// Path to reverseproxy-config.json
    pub reverseproxy_config_path: PathBuf,
}
