use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Configuration de l'autorité de certification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaConfig {
    /// Répertoire de stockage des certificats
    pub storage_path: String,
    /// Organisation (pour le certificat root)
    pub organization: String,
    /// Nom commun du certificat root
    pub common_name: String,
    /// Durée de validité du certificat root (en jours)
    pub root_validity_days: u32,
    /// Durée de validité des certificats serveur (en jours)
    pub cert_validity_days: u32,
    /// Seuil de renouvellement (en jours avant expiration)
    pub renewal_threshold_days: u32,
}

impl Default for CaConfig {
    fn default() -> Self {
        Self {
            storage_path: "/var/lib/server-dashboard/ca".to_string(),
            organization: "Homeroute Local CA".to_string(),
            common_name: "Homeroute Root CA".to_string(),
            root_validity_days: 3650,
            cert_validity_days: 365,
            renewal_threshold_days: 30,
        }
    }
}

/// Informations sur un certificat émis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateInfo {
    pub id: String,
    pub domains: Vec<String>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub serial_number: String,
    pub cert_path: String,
    pub key_path: String,
}

impl CertificateInfo {
    pub fn needs_renewal(&self, threshold_days: u32) -> bool {
        let now = Utc::now();
        let threshold = chrono::Duration::days(threshold_days as i64);
        self.expires_at - now < threshold
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

#[derive(Error, Debug)]
pub enum CaError {
    #[error("CA not initialized")]
    NotInitialized,

    #[error("CA already initialized")]
    AlreadyInitialized,

    #[error("Certificate generation failed: {0}")]
    GenerationFailed(String),

    #[error("Certificate not found: {0}")]
    CertificateNotFound(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Invalid domain: {0}")]
    InvalidDomain(String),

    #[error("Certificate parsing error: {0}")]
    ParsingError(String),
}

pub type CaResult<T> = Result<T, CaError>;
