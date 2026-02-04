use crate::cloudflare;
use crate::storage::AcmeStorage;
use crate::types::{AcmeConfig, AcmeError, AcmeResult, CertificateInfo, WildcardType};
use chrono::{Duration, Utc};
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, KeyAuthorization,
    NewAccount, NewOrder, OrderStatus,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

/// ACME certificate manager for Let's Encrypt
pub struct AcmeManager {
    config: AcmeConfig,
    storage: AcmeStorage,
    account: Arc<Mutex<Option<Account>>>,
}

impl AcmeManager {
    /// Create a new ACME manager
    pub fn new(config: AcmeConfig) -> Self {
        let storage = AcmeStorage::new(&config.storage_path);
        Self {
            config,
            storage,
            account: Arc::new(Mutex::new(None)),
        }
    }

    /// Initialize ACME: create storage dirs, load/create account
    pub async fn init(&self) -> AcmeResult<()> {
        self.storage.init()?;

        if self.config.cf_api_token.is_empty() || self.config.cf_zone_id.is_empty() {
            return Err(AcmeError::ConfigError(
                "Cloudflare credentials not configured".into(),
            ));
        }

        if self.config.base_domain.is_empty() {
            return Err(AcmeError::ConfigError("Base domain not configured".into()));
        }

        let account = if self.storage.is_initialized() {
            self.load_account().await?
        } else {
            self.create_account().await?
        };

        *self.account.lock().await = Some(account);
        info!("ACME manager initialized");
        Ok(())
    }

    /// Check if ACME is initialized
    pub fn is_initialized(&self) -> bool {
        self.storage.is_initialized()
    }

    /// Create a new Let's Encrypt account
    async fn create_account(&self) -> AcmeResult<Account> {
        info!("Creating new Let's Encrypt account");

        let email = if self.config.account_email.is_empty() {
            format!("admin@{}", self.config.base_domain)
        } else {
            self.config.account_email.clone()
        };

        let (account, credentials) = Account::create(
            &NewAccount {
                contact: &[&format!("mailto:{}", email)],
                terms_of_service_agreed: true,
                only_return_existing: false,
            },
            &self.config.directory_url,
            None,
        )
        .await
        .map_err(|e| AcmeError::ProtocolError(format!("Failed to create account: {}", e)))?;

        // Save account credentials
        let creds_json = serde_json::to_string_pretty(&credentials)?;
        self.storage.write_file(self.storage.account_path(), &creds_json)?;

        info!(email = %email, "Created new Let's Encrypt account");
        Ok(account)
    }

    /// Load existing Let's Encrypt account
    async fn load_account(&self) -> AcmeResult<Account> {
        debug!("Loading existing Let's Encrypt account");

        let creds_json = self.storage.read_file(self.storage.account_path())?;
        let credentials: AccountCredentials = serde_json::from_str(&creds_json)?;

        let account = Account::from_credentials(credentials)
            .await
            .map_err(|e| AcmeError::ProtocolError(format!("Failed to load account: {}", e)))?;

        info!("Loaded existing Let's Encrypt account");
        Ok(account)
    }

    /// Request a new wildcard certificate
    pub async fn request_wildcard(
        &self,
        wildcard_type: WildcardType,
    ) -> AcmeResult<CertificateInfo> {
        let account_guard = self.account.lock().await;
        let account = account_guard.as_ref().ok_or(AcmeError::NotInitialized)?;

        let wildcard_domain = wildcard_type.domain_pattern(&self.config.base_domain);

        info!(
            wildcard = %wildcard_domain,
            wildcard_type = ?wildcard_type,
            "Requesting wildcard certificate from Let's Encrypt"
        );

        let identifiers = vec![Identifier::Dns(wildcard_domain.clone())];

        // Create order
        let mut order = account
            .new_order(&NewOrder {
                identifiers: &identifiers,
            })
            .await
            .map_err(|e| AcmeError::ProtocolError(format!("Failed to create order: {}", e)))?;

        // Process authorizations (DNS-01 challenges)
        let authorizations = order
            .authorizations()
            .await
            .map_err(|e| AcmeError::ProtocolError(format!("Failed to get authorizations: {}", e)))?;

        let mut challenge_records: Vec<(String, String)> = Vec::new();

        for auth in authorizations {
            if auth.status == AuthorizationStatus::Valid {
                debug!("Authorization already valid, skipping");
                continue;
            }

            let challenge = auth
                .challenges
                .iter()
                .find(|c| c.r#type == ChallengeType::Dns01)
                .ok_or_else(|| {
                    AcmeError::ChallengeFailed("No DNS-01 challenge available".into())
                })?;

            // Build the challenge record name
            let domain_value = match &auth.identifier {
                Identifier::Dns(d) => d.clone(),
            };

            // For wildcard, the challenge is for the base domain without the *
            let challenge_domain = domain_value.trim_start_matches("*.");
            let dns_name = format!("_acme-challenge.{}", challenge_domain);

            let key_auth = order.key_authorization(challenge);
            let dns_value = key_auth.dns_value();

            debug!(dns_name = %dns_name, "Setting up DNS-01 challenge");

            // Create DNS record via Cloudflare
            let record_id = cloudflare::create_acme_challenge_record(
                &self.config.cf_api_token,
                &self.config.cf_zone_id,
                &dns_name,
                &dns_value,
            )
            .await
            .map_err(AcmeError::CloudflareError)?;

            challenge_records.push((dns_name.clone(), record_id));

            // Wait for DNS propagation
            info!("Waiting for DNS propagation (15 seconds)...");
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;

            // Tell ACME server to validate the challenge
            order
                .set_challenge_ready(&challenge.url)
                .await
                .map_err(|e| {
                    AcmeError::ProtocolError(format!("Failed to set challenge ready: {}", e))
                })?;
        }

        // Wait for order to be ready
        info!("Waiting for ACME order validation...");
        let mut attempts = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            order
                .refresh()
                .await
                .map_err(|e| AcmeError::ProtocolError(format!("Failed to refresh order: {}", e)))?;

            match order.state().status {
                OrderStatus::Ready => {
                    info!("Order is ready for finalization");
                    break;
                }
                OrderStatus::Invalid => {
                    // Cleanup DNS records before returning error
                    self.cleanup_challenge_records(&challenge_records).await;
                    return Err(AcmeError::ChallengeFailed(
                        "Order validation failed - order became invalid".into(),
                    ));
                }
                OrderStatus::Valid => {
                    info!("Order is already valid");
                    break;
                }
                status => {
                    debug!(status = ?status, attempt = attempts, "Order not ready yet");
                    attempts += 1;
                    if attempts > 60 {
                        // 5 minutes timeout
                        self.cleanup_challenge_records(&challenge_records).await;
                        return Err(AcmeError::ChallengeFailed(
                            "Timeout waiting for order validation".into(),
                        ));
                    }
                }
            }
        }

        // Cleanup DNS records
        self.cleanup_challenge_records(&challenge_records).await;

        // Generate CSR and finalize order
        info!("Generating CSR and finalizing order...");
        let mut params = rcgen::CertificateParams::new(vec![wildcard_domain.clone()])
            .map_err(|e| AcmeError::ProtocolError(format!("Failed to create cert params: {}", e)))?;
        params.distinguished_name = rcgen::DistinguishedName::new();

        let key_pair = rcgen::KeyPair::generate()
            .map_err(|e| AcmeError::ProtocolError(format!("Failed to generate key pair: {}", e)))?;

        let csr = params
            .serialize_request(&key_pair)
            .map_err(|e| AcmeError::ProtocolError(format!("Failed to create CSR: {}", e)))?;

        order
            .finalize(csr.der())
            .await
            .map_err(|e| AcmeError::ProtocolError(format!("Failed to finalize order: {}", e)))?;

        // Wait for certificate
        info!("Waiting for certificate...");
        let cert_chain = loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            match order.certificate().await {
                Ok(Some(chain)) => break chain,
                Ok(None) => {
                    debug!("Certificate not ready yet");
                    continue;
                }
                Err(e) => {
                    return Err(AcmeError::ProtocolError(format!(
                        "Failed to get certificate: {}",
                        e
                    )));
                }
            }
        };

        // Save certificate and key
        let cert_path = self.storage.cert_path(wildcard_type);
        let key_path = self.storage.key_path(wildcard_type);
        let chain_path = self.storage.chain_path(wildcard_type);

        self.storage.write_file(&cert_path, &cert_chain)?;
        self.storage.write_file(&key_path, &key_pair.serialize_pem())?;
        self.storage.write_file(&chain_path, &cert_chain)?;

        let now = Utc::now();
        let cert_info = CertificateInfo {
            id: wildcard_type.id().to_string(),
            wildcard_type,
            domains: vec![wildcard_domain.clone()],
            issued_at: now,
            expires_at: now + Duration::days(90), // Let's Encrypt certs are valid 90 days
            cert_path: cert_path.to_string_lossy().to_string(),
            key_path: key_path.to_string_lossy().to_string(),
        };

        // Update index
        let mut index = self.storage.load_index()?;
        index.retain(|c| c.wildcard_type != wildcard_type);
        index.push(cert_info.clone());
        self.storage.save_index(&index)?;

        info!(
            wildcard = %wildcard_domain,
            expires_at = %cert_info.expires_at,
            "Wildcard certificate issued successfully"
        );

        Ok(cert_info)
    }

    /// Cleanup challenge DNS records
    async fn cleanup_challenge_records(&self, records: &[(String, String)]) {
        for (dns_name, record_id) in records {
            if let Err(e) = cloudflare::delete_challenge_record(
                &self.config.cf_api_token,
                &self.config.cf_zone_id,
                record_id,
            )
            .await
            {
                warn!(dns_name = %dns_name, error = %e, "Failed to cleanup challenge record");
            }
        }
    }

    /// List all certificates
    pub fn list_certificates(&self) -> AcmeResult<Vec<CertificateInfo>> {
        self.storage.load_index()
    }

    /// Get a specific certificate by wildcard type
    pub fn get_certificate(&self, wildcard_type: WildcardType) -> AcmeResult<CertificateInfo> {
        let index = self.storage.load_index()?;
        index
            .into_iter()
            .find(|c| c.wildcard_type == wildcard_type)
            .ok_or_else(|| AcmeError::CertificateNotFound(wildcard_type.id().to_string()))
    }

    /// Get certificates that need renewal
    pub fn certificates_needing_renewal(&self) -> AcmeResult<Vec<CertificateInfo>> {
        let index = self.storage.load_index()?;
        Ok(index
            .into_iter()
            .filter(|c| c.needs_renewal(self.config.renewal_threshold_days))
            .collect())
    }

    /// Get certificate and key PEM for a wildcard type
    pub async fn get_cert_pem(&self, wildcard_type: WildcardType) -> AcmeResult<(String, String)> {
        let cert_path = self.storage.cert_path(wildcard_type);
        let key_path = self.storage.key_path(wildcard_type);

        if !cert_path.exists() || !key_path.exists() {
            return Err(AcmeError::CertificateNotFound(
                wildcard_type.id().to_string(),
            ));
        }

        let cert_pem = tokio::fs::read_to_string(&cert_path)
            .await
            .map_err(|e| AcmeError::IoError(e))?;
        let key_pem = tokio::fs::read_to_string(&key_path)
            .await
            .map_err(|e| AcmeError::IoError(e))?;

        Ok((cert_pem, key_pem))
    }

    /// Get the base domain
    pub fn base_domain(&self) -> &str {
        &self.config.base_domain
    }

    /// Get renewal threshold in days
    pub fn renewal_threshold_days(&self) -> u32 {
        self.config.renewal_threshold_days
    }
}
