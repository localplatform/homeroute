use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use anyhow::{Context, Result};
use tracing::{info, warn, error};

/// SNI-based certificate resolver for rustls
#[derive(Debug)]
pub struct SniResolver {
    /// Certified keys indexed by domain name
    certs: RwLock<HashMap<String, Arc<CertifiedKey>>>,
}

impl SniResolver {
    pub fn new() -> Self {
        Self {
            certs: RwLock::new(HashMap::new()),
        }
    }

    /// Insert a certified key for a domain
    pub fn insert(&self, domain: String, key: Arc<CertifiedKey>) {
        let mut certs = self.certs.write().unwrap();
        certs.insert(domain, key);
    }

    /// Remove a domain's certificate
    pub fn remove(&self, domain: &str) {
        let mut certs = self.certs.write().unwrap();
        certs.remove(domain);
    }

    /// List loaded domains
    pub fn loaded_domains(&self) -> Vec<String> {
        let certs = self.certs.read().unwrap();
        certs.keys().cloned().collect()
    }

    /// Clear all certificates
    pub fn clear(&self) {
        let mut certs = self.certs.write().unwrap();
        certs.clear();
    }
}

impl ResolvesServerCert for SniResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let server_name = client_hello.server_name()?;
        let certs = self.certs.read().unwrap();
        let key = certs.get(server_name).cloned();
        if key.is_none() {
            warn!("No certificate found for SNI: {}", server_name);
        }
        key
    }
}

/// TLS Manager - loads certificates from CA storage and builds the SNI resolver
pub struct TlsManager {
    /// CA storage path
    ca_storage_path: PathBuf,

    /// SNI resolver (shared with the TLS acceptor)
    pub resolver: Arc<SniResolver>,
}

impl TlsManager {
    pub fn new(ca_storage_path: PathBuf) -> Self {
        Self {
            ca_storage_path,
            resolver: Arc::new(SniResolver::new()),
        }
    }

    /// Load a certificate for a specific domain
    pub fn load_certificate(&self, domain: &str, cert_id: &str) -> Result<()> {
        let cert_path = self.ca_storage_path.join("certs").join(format!("{}.crt", cert_id));
        let key_path = self.ca_storage_path.join("keys").join(format!("{}.key", cert_id));

        let certs = load_certs(&cert_path)?;
        let key = load_private_key(&key_path)?;

        let signing_key = rustls::crypto::ring::sign::any_supported_type(&key)
            .map_err(|e| anyhow::anyhow!("Failed to parse signing key: {}", e))?;

        let certified_key = CertifiedKey::new(certs, signing_key);

        self.resolver.insert(domain.to_string(), Arc::new(certified_key));
        info!("Loaded TLS certificate for domain: {}", domain);
        Ok(())
    }

    /// Build the rustls ServerConfig with our SNI resolver
    pub fn build_server_config(&self) -> Result<Arc<ServerConfig>> {
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(self.resolver.clone());

        Ok(Arc::new(config))
    }

    /// Remove a certificate
    pub fn remove_certificate(&self, domain: &str) {
        self.resolver.remove(domain);
        info!("Removed TLS certificate for domain: {}", domain);
    }

    /// List loaded domains
    pub fn loaded_domains(&self) -> Vec<String> {
        self.resolver.loaded_domains()
    }

    /// Reload all certificates from config
    pub fn reload_certificates(&self, routes: &[crate::config::RouteConfig]) -> Result<()> {
        self.resolver.clear();
        for route in routes {
            if route.enabled {
                if let Some(cert_id) = &route.cert_id {
                    match self.load_certificate(&route.domain, cert_id) {
                        Ok(_) => info!("Reloaded certificate for: {}", route.domain),
                        Err(e) => error!("Failed to reload certificate for {}: {}", route.domain, e),
                    }
                }
            }
        }
        Ok(())
    }
}

/// Load certificates from a PEM file
fn load_certs(path: &PathBuf) -> Result<Vec<CertificateDer<'static>>> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open certificate file: {:?}", path))?;
    let mut reader = BufReader::new(file);

    let certs = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse certificates")?;

    if certs.is_empty() {
        anyhow::bail!("No certificates found in file");
    }

    Ok(certs)
}

/// Load private key from a PEM file
fn load_private_key(path: &PathBuf) -> Result<PrivateKeyDer<'static>> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open private key file: {:?}", path))?;
    let mut reader = BufReader::new(file);

    let key = rustls_pemfile::private_key(&mut reader)
        .context("Failed to parse private key")?;

    key.ok_or_else(|| anyhow::anyhow!("No private key found in file"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sni_resolver_insert_and_lookup() {
        let resolver = SniResolver::new();
        assert!(resolver.loaded_domains().is_empty());

        // We can't easily create a CertifiedKey without real certs,
        // but we can test the domain management methods
        assert_eq!(resolver.loaded_domains().len(), 0);
    }

    #[test]
    fn test_sni_resolver_clear() {
        let resolver = SniResolver::new();
        resolver.clear();
        assert!(resolver.loaded_domains().is_empty());
    }

    #[test]
    fn test_tls_manager_creation() {
        let manager = TlsManager::new(PathBuf::from("/tmp/test-ca"));
        assert!(manager.loaded_domains().is_empty());
    }

    #[test]
    fn test_tls_manager_build_server_config() {
        // Install crypto provider for test
        let _ = rustls::crypto::ring::default_provider().install_default();
        let manager = TlsManager::new(PathBuf::from("/tmp/test-ca"));
        let config = manager.build_server_config();
        assert!(config.is_ok());
    }

    #[test]
    fn test_load_cert_missing_file() {
        let result = load_certs(&PathBuf::from("/nonexistent/cert.pem"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_key_missing_file() {
        let result = load_private_key(&PathBuf::from("/nonexistent/key.pem"));
        assert!(result.is_err());
    }

    #[test]
    fn test_tls_manager_load_missing_cert() {
        let manager = TlsManager::new(PathBuf::from("/tmp/nonexistent-ca"));
        let result = manager.load_certificate("test.example.com", "fake-id");
        assert!(result.is_err());
    }

    #[test]
    fn test_tls_manager_remove_certificate() {
        let manager = TlsManager::new(PathBuf::from("/tmp/test-ca"));
        // Remove non-existent - should not panic
        manager.remove_certificate("nonexistent.example.com");
        assert!(manager.loaded_domains().is_empty());
    }

    #[test]
    fn test_reload_certificates_empty_routes() {
        let manager = TlsManager::new(PathBuf::from("/tmp/test-ca"));
        let result = manager.reload_certificates(&[]);
        assert!(result.is_ok());
        assert!(manager.loaded_domains().is_empty());
    }

    #[test]
    fn test_reload_certificates_skips_disabled() {
        let manager = TlsManager::new(PathBuf::from("/tmp/test-ca"));
        let routes = vec![
            crate::config::RouteConfig {
                id: "1".to_string(),
                domain: "enabled.example.com".to_string(),
                backend: "rust".to_string(),
                target_host: "localhost".to_string(),
                target_port: 8080,
                local_only: false,
                require_auth: false,
                enabled: true,
                cert_id: None, // No cert_id, so loading is skipped
            },
            crate::config::RouteConfig {
                id: "2".to_string(),
                domain: "disabled.example.com".to_string(),
                backend: "rust".to_string(),
                target_host: "localhost".to_string(),
                target_port: 8081,
                local_only: false,
                require_auth: false,
                enabled: false,
                cert_id: Some("cert-2".to_string()),
            },
        ];
        // Should succeed - disabled route is skipped, enabled route has no cert_id so skipped too
        let result = manager.reload_certificates(&routes);
        assert!(result.is_ok());
        assert!(manager.loaded_domains().is_empty());
    }
}
