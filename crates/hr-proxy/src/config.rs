use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Configuration principale du reverse proxy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// Port d'écoute HTTP (pour redirection HTTPS)
    #[serde(default = "default_http_port")]
    pub http_port: u16,

    /// Port d'écoute HTTPS
    #[serde(default = "default_https_port")]
    pub https_port: u16,

    /// Domaine de base
    pub base_domain: String,

    /// Mode TLS : "local-ca" ou "letsencrypt" (futur)
    #[serde(default = "default_tls_mode")]
    pub tls_mode: String,

    /// Chemin vers le stockage de la CA locale
    #[serde(default = "default_ca_path")]
    pub ca_storage_path: PathBuf,


    /// Routes configurées
    #[serde(default)]
    pub routes: Vec<RouteConfig>,

    /// Chemin du fichier de log d'accès JSON (optionnel)
    #[serde(default)]
    pub access_log_path: Option<String>,
}

fn default_http_port() -> u16 { 80 }
fn default_https_port() -> u16 { 443 }
fn default_tls_mode() -> String { "local-ca".to_string() }
fn default_ca_path() -> PathBuf { PathBuf::from("/var/lib/server-dashboard/ca") }
/// Configuration d'une route
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteConfig {
    /// Identifiant unique
    pub id: String,

    /// Domaine (ex: "test.mynetwk.biz")
    pub domain: String,

    /// Backend : "caddy" ou "rust"
    #[serde(default = "default_backend")]
    pub backend: String,

    /// Host cible
    pub target_host: String,

    /// Port cible
    pub target_port: u16,

    /// Restreindre aux IPs locales uniquement
    #[serde(default)]
    pub local_only: bool,

    /// Requérir authentification
    #[serde(default)]
    pub require_auth: bool,

    /// Actif ou non
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// ID du certificat CA (auto-généré si vide)
    #[serde(default)]
    pub cert_id: Option<String>,
}

fn default_backend() -> String { "rust".to_string() }
fn default_enabled() -> bool { true }

impl ProxyConfig {
    /// Charge la configuration depuis un fichier JSON
    pub fn load_from_file(path: &PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: ProxyConfig = serde_json::from_str(&content)?;
        Ok(config)
    }

    /// Sauvegarde la configuration dans un fichier JSON
    pub fn save_to_file(&self, path: &PathBuf) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Retourne uniquement les routes actives
    pub fn active_routes(&self) -> Vec<&RouteConfig> {
        self.routes
            .iter()
            .filter(|r| r.enabled)
            .collect()
    }

    /// Retourne les routes groupées par domaine
    pub fn routes_by_domain(&self) -> HashMap<String, &RouteConfig> {
        self.active_routes()
            .into_iter()
            .map(|r| (r.domain.clone(), r))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ProxyConfig {
            http_port: 80,
            https_port: 443,
            base_domain: "example.com".to_string(),
            tls_mode: "local-ca".to_string(),
            ca_storage_path: PathBuf::from("/var/lib/server-dashboard/ca"),
            routes: vec![],
            access_log_path: None,
        };

        assert_eq!(config.https_port, 443);
        assert_eq!(config.base_domain, "example.com");
    }

    #[test]
    fn test_active_routes_filter() {
        let config = ProxyConfig {
            http_port: 80,
            https_port: 443,
            base_domain: "example.com".to_string(),
            tls_mode: "local-ca".to_string(),
            ca_storage_path: PathBuf::from("/var/lib/server-dashboard/ca"),
            routes: vec![
                RouteConfig {
                    id: "1".to_string(),
                    domain: "test1.example.com".to_string(),
                    backend: "rust".to_string(),
                    target_host: "localhost".to_string(),
                    target_port: 8080,
                    local_only: false,
                    require_auth: false,
                    enabled: true,
                    cert_id: None,
                },
                RouteConfig {
                    id: "2".to_string(),
                    domain: "test2.example.com".to_string(),
                    backend: "caddy".to_string(),
                    target_host: "localhost".to_string(),
                    target_port: 8081,
                    local_only: false,
                    require_auth: false,
                    enabled: true,
                    cert_id: None,
                },
                RouteConfig {
                    id: "3".to_string(),
                    domain: "test3.example.com".to_string(),
                    backend: "rust".to_string(),
                    target_host: "localhost".to_string(),
                    target_port: 8082,
                    local_only: false,
                    require_auth: false,
                    enabled: false,
                    cert_id: None,
                },
            ],
            access_log_path: None,
        };

        let active = config.active_routes();
        // Both enabled routes are returned regardless of backend
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].domain, "test1.example.com");
        assert_eq!(active[1].domain, "test2.example.com");
    }
}
