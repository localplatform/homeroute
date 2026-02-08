use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Configuration principale chargée depuis les variables d'environnement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvConfig {
    /// Port de l'API interne (inutilisé dans le binaire unifié, gardé pour compat)
    pub api_port: u16,
    /// Domaine de base pour tous les sous-domaines
    pub base_domain: String,
    /// Répertoire des données d'auth (sessions, users)
    pub auth_data_dir: PathBuf,
    /// Cloudflare DDNS
    pub cf_api_token: Option<String>,
    pub cf_zone_id: Option<String>,
    pub cf_record_name: Option<String>,
    pub cf_interface: String,
    pub cf_proxied: bool,
    pub ddns_cron: String,
    /// Chemins de configuration des services
    pub proxy_config_path: PathBuf,
    pub dns_dhcp_config_path: PathBuf,
    pub reverseproxy_config_path: PathBuf,
    /// Répertoire ACME (Let's Encrypt)
    pub acme_storage_path: PathBuf,
    /// Email pour le compte ACME
    pub acme_email: Option<String>,
    /// Utiliser l'environnement de staging Let's Encrypt
    pub acme_staging: bool,
    /// Répertoire des données applicatives
    pub data_dir: PathBuf,
    /// Répertoire des logs
    pub log_dir: PathBuf,
    /// Chemin du frontend buildé
    pub web_dist_path: PathBuf,
    /// Cloud Relay
    pub cloud_relay_enabled: bool,
    pub cloud_relay_host: Option<String>,
    pub cloud_relay_quic_port: u16,
    pub cloud_relay_ssh_user: Option<String>,
    pub cloud_relay_ssh_port: u16,
}

impl Default for EnvConfig {
    fn default() -> Self {
        Self {
            api_port: 4000,
            base_domain: "localhost".to_string(),
            auth_data_dir: PathBuf::from("/opt/homeroute/data"),
            cf_api_token: None,
            cf_zone_id: None,
            cf_record_name: None,
            cf_interface: "eno1".to_string(),
            cf_proxied: true,
            ddns_cron: "*/2 * * * *".to_string(),
            proxy_config_path: PathBuf::from(
                "/var/lib/server-dashboard/rust-proxy-config.json",
            ),
            dns_dhcp_config_path: PathBuf::from(
                "/var/lib/server-dashboard/dns-dhcp-config.json",
            ),
            reverseproxy_config_path: PathBuf::from(
                "/var/lib/server-dashboard/reverseproxy-config.json",
            ),
            acme_storage_path: PathBuf::from("/var/lib/server-dashboard/acme"),
            acme_email: None,
            acme_staging: false,
            data_dir: PathBuf::from("/opt/homeroute/data"),
            log_dir: PathBuf::from("/var/log/homeroute"),
            web_dist_path: PathBuf::from("/opt/homeroute/web/dist"),
            cloud_relay_enabled: false,
            cloud_relay_host: None,
            cloud_relay_quic_port: 4443,
            cloud_relay_ssh_user: None,
            cloud_relay_ssh_port: 22,
        }
    }
}

impl EnvConfig {
    /// Charge la configuration depuis les variables d'environnement
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(v) = std::env::var("API_PORT") {
            if let Ok(port) = v.parse() {
                config.api_port = port;
            }
        }
        if let Ok(v) = std::env::var("BASE_DOMAIN") {
            config.base_domain = v;
        }
        if let Ok(v) = std::env::var("AUTH_DATA_DIR") {
            config.auth_data_dir = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("CF_API_TOKEN") {
            config.cf_api_token = Some(v);
        }
        if let Ok(v) = std::env::var("CF_ZONE_ID") {
            config.cf_zone_id = Some(v);
        }
        if let Ok(v) = std::env::var("CF_RECORD_NAME") {
            config.cf_record_name = Some(v);
        }
        if let Ok(v) = std::env::var("CF_INTERFACE") {
            config.cf_interface = v;
        }
        if let Ok(v) = std::env::var("CF_PROXIED") {
            config.cf_proxied = v.to_lowercase() != "false" && v != "0";
        }
        if let Ok(v) = std::env::var("DDNS_CRON") {
            config.ddns_cron = v;
        }
        if let Ok(v) = std::env::var("DATA_DIR") {
            config.data_dir = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("LOG_DIR") {
            config.log_dir = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("WEB_DIST_PATH") {
            config.web_dist_path = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("ACME_STORAGE_PATH") {
            config.acme_storage_path = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("ACME_EMAIL") {
            config.acme_email = Some(v);
        }
        if let Ok(v) = std::env::var("ACME_STAGING") {
            config.acme_staging = v == "1" || v.to_lowercase() == "true";
        }
        if let Ok(v) = std::env::var("CLOUD_RELAY_ENABLED") {
            config.cloud_relay_enabled = v == "1" || v.to_lowercase() == "true";
        }
        if let Ok(v) = std::env::var("CLOUD_RELAY_HOST") {
            config.cloud_relay_host = Some(v);
        }
        if let Ok(v) = std::env::var("CLOUD_RELAY_QUIC_PORT") {
            if let Ok(port) = v.parse() {
                config.cloud_relay_quic_port = port;
            }
        }
        if let Ok(v) = std::env::var("CLOUD_RELAY_SSH_USER") {
            config.cloud_relay_ssh_user = Some(v);
        }
        if let Ok(v) = std::env::var("CLOUD_RELAY_SSH_PORT") {
            if let Ok(port) = v.parse() {
                config.cloud_relay_ssh_port = port;
            }
        }

        config
    }

    /// Charge le fichier .env puis les variables d'environnement
    pub fn load(env_file: Option<&Path>) -> Self {
        if let Some(path) = env_file {
            load_dotenv(path);
        } else {
            // Chercher .env dans le répertoire courant ou /opt/homeroute
            let candidates = [
                PathBuf::from("/opt/homeroute/.env"),
                PathBuf::from(".env"),
            ];
            for candidate in &candidates {
                if candidate.exists() {
                    load_dotenv(candidate);
                    break;
                }
            }
        }

        Self::from_env()
    }
}

/// Charge un fichier .env basique (KEY=VALUE par ligne)
fn load_dotenv(path: &Path) {
    if let Ok(content) = std::fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim().trim_matches('"').trim_matches('\'');
                if std::env::var(key).is_err() {
                    // SAFETY: called before spawning any threads (single-threaded init)
                    unsafe { std::env::set_var(key, value) };
                }
            }
        }
    }
}
