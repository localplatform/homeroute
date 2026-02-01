use crate::types::{CaResult, CertificateInfo};
use std::fs;
use std::path::{Path, PathBuf};

/// Gestion du stockage des certificats sur le système de fichiers
pub struct CaStorage {
    base_path: PathBuf,
}

impl CaStorage {
    pub fn new<P: AsRef<Path>>(base_path: P) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
        }
    }

    pub fn init(&self) -> CaResult<()> {
        fs::create_dir_all(&self.base_path)?;
        fs::create_dir_all(self.base_path.join("certs"))?;
        fs::create_dir_all(self.base_path.join("keys"))?;
        Ok(())
    }

    pub fn root_cert_path(&self) -> PathBuf {
        self.base_path.join("root-ca.crt")
    }

    pub fn root_key_path(&self) -> PathBuf {
        self.base_path.join("root-ca.key")
    }

    pub fn index_path(&self) -> PathBuf {
        self.base_path.join("index.json")
    }

    pub fn cert_path(&self, id: &str) -> PathBuf {
        self.base_path.join("certs").join(format!("{}.crt", id))
    }

    pub fn key_path(&self, id: &str) -> PathBuf {
        self.base_path.join("keys").join(format!("{}.key", id))
    }

    pub fn is_initialized(&self) -> bool {
        self.root_cert_path().exists() && self.root_key_path().exists()
    }

    pub fn load_index(&self) -> CaResult<Vec<CertificateInfo>> {
        let index_path = self.index_path();
        if !index_path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(index_path)?;
        let index: Vec<CertificateInfo> = serde_json::from_str(&content)?;
        Ok(index)
    }

    pub fn save_index(&self, index: &[CertificateInfo]) -> CaResult<()> {
        let content = serde_json::to_string_pretty(index)?;
        fs::write(self.index_path(), content)?;
        Ok(())
    }

    pub fn write_file<P: AsRef<Path>>(&self, path: P, content: &str) -> CaResult<()> {
        fs::write(path.as_ref(), content)?;
        Ok(())
    }

    pub fn read_file<P: AsRef<Path>>(&self, path: P) -> CaResult<String> {
        let content = fs::read_to_string(path.as_ref())?;
        Ok(content)
    }

    pub fn delete_certificate(&self, id: &str) -> CaResult<()> {
        let cert_path = self.cert_path(id);
        let key_path = self.key_path(id);

        if cert_path.exists() {
            fs::remove_file(cert_path)?;
        }
        if key_path.exists() {
            fs::remove_file(key_path)?;
        }

        Ok(())
    }
}
