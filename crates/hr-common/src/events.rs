use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Bus d'événements pour la communication inter-services
pub struct EventBus {
    /// Événements trafic HTTP (proxy → analytics, proxy → websocket)
    pub http_traffic: broadcast::Sender<HttpTrafficEvent>,
    /// Événements requêtes DNS (dns → analytics, dns → websocket)
    pub dns_traffic: broadcast::Sender<DnsTrafficEvent>,
    /// Métriques réseau (capture → websocket)
    pub network_metrics: broadcast::Sender<NetworkMetricsEvent>,
    /// Changements de statut serveurs (monitoring → websocket)
    pub server_status: broadcast::Sender<ServerStatusEvent>,
    /// Notifications de changement de config (API → services pour reload)
    pub config_changed: broadcast::Sender<ConfigChangeEvent>,
    /// System update events (updates → websocket)
    pub updates: broadcast::Sender<UpdateEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            http_traffic: broadcast::channel(1024).0,
            dns_traffic: broadcast::channel(1024).0,
            network_metrics: broadcast::channel(256).0,
            server_status: broadcast::channel(64).0,
            config_changed: broadcast::channel(16).0,
            updates: broadcast::channel(256).0,
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpTrafficEvent {
    pub timestamp: String,
    pub client_ip: String,
    pub host: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration_ms: u64,
    pub user_agent: String,
    pub response_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsTrafficEvent {
    pub timestamp: String,
    pub client_ip: String,
    pub domain: String,
    pub query_type: String,
    pub blocked: bool,
    pub cached: bool,
    pub response_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkMetricsEvent {
    pub timestamp: String,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub packets_in: u64,
    pub packets_out: u64,
    pub bandwidth_mbps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatusEvent {
    pub server_id: String,
    pub status: String,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConfigChangeEvent {
    ProxyRoutes,
    DnsDhcp,
    Adblock,
    Users,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum UpdateEvent {
    Started,
    Phase { phase: String, message: String },
    Output { line: String },
    AptComplete { packages: Vec<serde_json::Value>, security_count: usize },
    SnapComplete { snaps: Vec<serde_json::Value> },
    NeedrestartComplete(serde_json::Value),
    Complete { success: bool, summary: serde_json::Value, duration: u64 },
    Cancelled,
    Error { error: String },
    UpgradeStarted { upgrade_type: String },
    UpgradeOutput { line: String },
    UpgradeComplete { upgrade_type: String, success: bool, duration: u64, error: Option<String> },
    UpgradeCancelled,
}
