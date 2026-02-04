use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Configuration for ACME certificate management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcmeConfig {
    /// Storage path for ACME data
    pub storage_path: String,
    /// Cloudflare API token for DNS-01 challenges
    pub cf_api_token: String,
    /// Cloudflare Zone ID
    pub cf_zone_id: String,
    /// Base domain (e.g., "mynetwk.biz")
    pub base_domain: String,
    /// Let's Encrypt directory URL (production or staging)
    pub directory_url: String,
    /// Account email for Let's Encrypt
    pub account_email: String,
    /// Days before expiry to trigger renewal
    pub renewal_threshold_days: u32,
}

impl Default for AcmeConfig {
    fn default() -> Self {
        Self {
            storage_path: "/var/lib/server-dashboard/acme".to_string(),
            cf_api_token: String::new(),
            cf_zone_id: String::new(),
            base_domain: String::new(),
            directory_url: "https://acme-v02.api.letsencrypt.org/directory".to_string(),
            account_email: String::new(),
            renewal_threshold_days: 30,
        }
    }
}

/// Type of wildcard certificate
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WildcardType {
    /// *.mynetwk.biz - main apps
    Main,
    /// *.code.mynetwk.biz - code-server IDEs
    Code,
}

impl WildcardType {
    /// Get the domain pattern for this wildcard type
    pub fn domain_pattern(&self, base_domain: &str) -> String {
        match self {
            Self::Main => format!("*.{}", base_domain),
            Self::Code => format!("*.code.{}", base_domain),
        }
    }

    /// Get the unique ID for this wildcard type
    pub fn id(&self) -> &'static str {
        match self {
            Self::Main => "wildcard-main",
            Self::Code => "wildcard-code",
        }
    }

    /// Get display name
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Main => "Applications",
            Self::Code => "Code Server",
        }
    }

    /// Determine wildcard type from a domain
    pub fn from_domain(domain: &str) -> Self {
        if domain.contains(".code.") {
            Self::Code
        } else {
            Self::Main
        }
    }
}

/// Information about an issued certificate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateInfo {
    pub id: String,
    pub wildcard_type: WildcardType,
    pub domains: Vec<String>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub cert_path: String,
    pub key_path: String,
}

impl CertificateInfo {
    /// Check if certificate needs renewal
    pub fn needs_renewal(&self, threshold_days: u32) -> bool {
        let now = Utc::now();
        let threshold = chrono::Duration::days(threshold_days as i64);
        self.expires_at - now < threshold
    }

    /// Check if certificate is expired
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Get days until expiration
    pub fn days_until_expiry(&self) -> i64 {
        let now = Utc::now();
        (self.expires_at - now).num_days()
    }
}

#[derive(Error, Debug)]
pub enum AcmeError {
    #[error("ACME not initialized")]
    NotInitialized,

    #[error("ACME challenge failed: {0}")]
    ChallengeFailed(String),

    #[error("Certificate not found: {0}")]
    CertificateNotFound(String),

    #[error("Cloudflare API error: {0}")]
    CloudflareError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("ACME protocol error: {0}")]
    ProtocolError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),
}

pub type AcmeResult<T> = Result<T, AcmeError>;
