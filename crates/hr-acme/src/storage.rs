use crate::types::{AcmeResult, CertificateInfo, WildcardType};
use std::fs;
use std::path::{Path, PathBuf};

/// ACME certificate storage management
pub struct AcmeStorage {
    base_path: PathBuf,
}

impl AcmeStorage {
    pub fn new<P: AsRef<Path>>(base_path: P) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
        }
    }

    /// Initialize storage directories
    pub fn init(&self) -> AcmeResult<()> {
        fs::create_dir_all(&self.base_path)?;
        fs::create_dir_all(self.base_path.join("certs"))?;
        fs::create_dir_all(self.base_path.join("keys"))?;
        Ok(())
    }

    /// Path to account credentials file
    pub fn account_path(&self) -> PathBuf {
        self.base_path.join("account.json")
    }

    /// Path to certificate file for a wildcard type
    pub fn cert_path(&self, wildcard_type: WildcardType) -> PathBuf {
        self.base_path
            .join("certs")
            .join(format!("{}.crt", wildcard_type.id()))
    }

    /// Path to private key file for a wildcard type
    pub fn key_path(&self, wildcard_type: WildcardType) -> PathBuf {
        self.base_path
            .join("keys")
            .join(format!("{}.key", wildcard_type.id()))
    }

    /// Path to full chain certificate
    pub fn chain_path(&self, wildcard_type: WildcardType) -> PathBuf {
        self.base_path
            .join("certs")
            .join(format!("{}-chain.crt", wildcard_type.id()))
    }

    /// Path to certificate index file
    pub fn index_path(&self) -> PathBuf {
        self.base_path.join("index.json")
    }

    /// Check if ACME account is initialized
    pub fn is_initialized(&self) -> bool {
        self.account_path().exists()
    }

    /// Load certificate index
    pub fn load_index(&self) -> AcmeResult<Vec<CertificateInfo>> {
        let index_path = self.index_path();
        if !index_path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(index_path)?;
        let index: Vec<CertificateInfo> = serde_json::from_str(&content)?;
        Ok(index)
    }

    /// Save certificate index atomically
    pub fn save_index(&self, index: &[CertificateInfo]) -> AcmeResult<()> {
        let content = serde_json::to_string_pretty(index)?;
        let index_path = self.index_path();
        let temp_path = index_path.with_extension("json.tmp");

        // Write to temporary file first
        fs::write(&temp_path, &content)?;

        // Atomic rename
        fs::rename(&temp_path, &index_path)?;

        Ok(())
    }

    /// Write a file
    pub fn write_file<P: AsRef<Path>>(&self, path: P, content: &str) -> AcmeResult<()> {
        fs::write(path.as_ref(), content)?;
        Ok(())
    }

    /// Read a file
    pub fn read_file<P: AsRef<Path>>(&self, path: P) -> AcmeResult<String> {
        let content = fs::read_to_string(path.as_ref())?;
        Ok(content)
    }

    /// Check if certificate files exist
    pub fn cert_exists(&self, wildcard_type: WildcardType) -> bool {
        self.cert_path(wildcard_type).exists() && self.key_path(wildcard_type).exists()
    }
}
