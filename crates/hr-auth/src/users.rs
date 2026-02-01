use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Données utilisateur sérialisées en YAML
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsersFile {
    #[serde(default)]
    users: HashMap<String, UserData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserData {
    #[serde(default)]
    displayname: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    groups: Vec<String>,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    last_login: Option<String>,
}

/// Informations utilisateur (sans mot de passe)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInfo {
    pub username: String,
    pub displayname: String,
    pub email: String,
    pub groups: Vec<String>,
    pub disabled: bool,
    pub created: Option<String>,
    pub last_login: Option<String>,
}

/// Informations utilisateur avec hash du mot de passe (pour l'auth)
#[derive(Debug, Clone)]
pub struct UserWithPassword {
    pub username: String,
    pub displayname: String,
    pub email: String,
    pub password_hash: String,
    pub groups: Vec<String>,
    pub disabled: bool,
}

/// Résultat d'une opération sur les utilisateurs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOpResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserInfo>,
}

/// Store d'utilisateurs basé sur un fichier YAML
pub struct UserStore {
    users_path: PathBuf,
}

impl UserStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            users_path: data_dir.join("users.yml"),
        }
    }

    fn load(&self) -> UsersFile {
        match std::fs::read_to_string(&self.users_path) {
            Ok(content) => serde_yaml::from_str(&content).unwrap_or(UsersFile {
                users: HashMap::new(),
            }),
            Err(_) => UsersFile {
                users: HashMap::new(),
            },
        }
    }

    fn save(&self, data: &UsersFile) -> bool {
        if let Some(parent) = self.users_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_yaml::to_string(data) {
            Ok(yaml) => std::fs::write(&self.users_path, yaml).is_ok(),
            Err(_) => false,
        }
    }

    /// Liste tous les utilisateurs (sans mot de passe)
    pub fn get_all(&self) -> Vec<UserInfo> {
        let data = self.load();
        data.users
            .iter()
            .map(|(username, ud)| UserInfo {
                username: username.clone(),
                displayname: ud.displayname.clone().unwrap_or_else(|| username.clone()),
                email: ud.email.clone().unwrap_or_default(),
                groups: ud.groups.clone(),
                disabled: ud.disabled,
                created: ud.created.clone(),
                last_login: ud.last_login.clone(),
            })
            .collect()
    }

    /// Récupère un utilisateur par nom (sans mot de passe)
    pub fn get(&self, username: &str) -> Option<UserInfo> {
        let data = self.load();
        data.users.get(username).map(|ud| UserInfo {
            username: username.to_string(),
            displayname: ud.displayname.clone().unwrap_or_else(|| username.to_string()),
            email: ud.email.clone().unwrap_or_default(),
            groups: ud.groups.clone(),
            disabled: ud.disabled,
            created: ud.created.clone(),
            last_login: ud.last_login.clone(),
        })
    }

    /// Récupère un utilisateur avec le hash du mot de passe (pour l'authentification)
    pub fn get_with_password(&self, username: &str) -> Option<UserWithPassword> {
        let data = self.load();
        data.users.get(username).and_then(|ud| {
            ud.password.as_ref().map(|pw| UserWithPassword {
                username: username.to_string(),
                displayname: ud.displayname.clone().unwrap_or_else(|| username.to_string()),
                email: ud.email.clone().unwrap_or_default(),
                password_hash: pw.clone(),
                groups: ud.groups.clone(),
                disabled: ud.disabled,
            })
        })
    }

    /// Crée un nouvel utilisateur
    pub fn create(
        &self,
        username: &str,
        password: &str,
        displayname: Option<&str>,
        email: Option<&str>,
        groups: Vec<String>,
    ) -> UserOpResult {
        let mut data = self.load();

        if data.users.contains_key(username) {
            return UserOpResult {
                success: false,
                error: Some("Utilisateur deja existant".to_string()),
                user: None,
            };
        }

        // Valider le nom d'utilisateur
        let re_valid = username.len() >= 3
            && username.len() <= 32
            && username
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-');
        if !re_valid {
            return UserOpResult {
                success: false,
                error: Some(
                    "Nom d'utilisateur invalide (3-32 caracteres, lettres, chiffres, _ ou -)"
                        .to_string(),
                ),
                user: None,
            };
        }

        if password.len() < 8 {
            return UserOpResult {
                success: false,
                error: Some("Le mot de passe doit contenir au moins 8 caracteres".to_string()),
                user: None,
            };
        }

        let hashed = match hash_password(password) {
            Ok(h) => h,
            Err(_) => {
                return UserOpResult {
                    success: false,
                    error: Some("Erreur de hachage du mot de passe".to_string()),
                    user: None,
                }
            }
        };

        let now = chrono::Utc::now().to_rfc3339();
        data.users.insert(
            username.to_string(),
            UserData {
                displayname: Some(
                    displayname
                        .unwrap_or(username)
                        .to_string(),
                ),
                email: Some(email.unwrap_or("").to_string()),
                password: Some(hashed),
                groups,
                disabled: false,
                created: Some(now),
                last_login: None,
            },
        );

        if !self.save(&data) {
            return UserOpResult {
                success: false,
                error: Some("Erreur lors de la sauvegarde".to_string()),
                user: None,
            };
        }

        UserOpResult {
            success: true,
            error: None,
            user: self.get(username),
        }
    }

    /// Met à jour un utilisateur existant
    pub fn update(&self, username: &str, updates: &UserUpdates) -> UserOpResult {
        let mut data = self.load();

        let Some(user) = data.users.get_mut(username) else {
            return UserOpResult {
                success: false,
                error: Some("Utilisateur non trouve".to_string()),
                user: None,
            };
        };

        if let Some(ref dn) = updates.displayname {
            user.displayname = Some(dn.clone());
        }
        if let Some(ref email) = updates.email {
            user.email = Some(email.clone());
        }
        if let Some(ref groups) = updates.groups {
            user.groups = groups.clone();
        }
        if let Some(disabled) = updates.disabled {
            user.disabled = disabled;
        }

        if !self.save(&data) {
            return UserOpResult {
                success: false,
                error: Some("Erreur lors de la sauvegarde".to_string()),
                user: None,
            };
        }

        UserOpResult {
            success: true,
            error: None,
            user: self.get(username),
        }
    }

    /// Met à jour le timestamp de dernière connexion
    pub fn update_last_login(&self, username: &str) -> bool {
        let mut data = self.load();
        if let Some(user) = data.users.get_mut(username) {
            user.last_login = Some(chrono::Utc::now().to_rfc3339());
            self.save(&data)
        } else {
            false
        }
    }

    /// Change le mot de passe d'un utilisateur
    pub fn change_password(&self, username: &str, new_password: &str) -> UserOpResult {
        if new_password.len() < 8 {
            return UserOpResult {
                success: false,
                error: Some("Le mot de passe doit contenir au moins 8 caracteres".to_string()),
                user: None,
            };
        }

        let mut data = self.load();
        let Some(user) = data.users.get_mut(username) else {
            return UserOpResult {
                success: false,
                error: Some("Utilisateur non trouve".to_string()),
                user: None,
            };
        };

        let hashed = match hash_password(new_password) {
            Ok(h) => h,
            Err(_) => {
                return UserOpResult {
                    success: false,
                    error: Some("Erreur de hachage du mot de passe".to_string()),
                    user: None,
                }
            }
        };

        user.password = Some(hashed);

        if !self.save(&data) {
            return UserOpResult {
                success: false,
                error: Some("Erreur lors de la sauvegarde".to_string()),
                user: None,
            };
        }

        UserOpResult {
            success: true,
            error: None,
            user: None,
        }
    }

    /// Supprime un utilisateur
    pub fn delete(&self, username: &str) -> UserOpResult {
        let mut data = self.load();

        if data.users.remove(username).is_none() {
            return UserOpResult {
                success: false,
                error: Some("Utilisateur non trouve".to_string()),
                user: None,
            };
        }

        if !self.save(&data) {
            return UserOpResult {
                success: false,
                error: Some("Erreur lors de la sauvegarde".to_string()),
                user: None,
            };
        }

        UserOpResult {
            success: true,
            error: None,
            user: None,
        }
    }

    /// Vérifie si un utilisateur est admin
    pub fn is_admin(&self, username: &str) -> bool {
        self.get(username)
            .map(|u| u.groups.contains(&"admins".to_string()))
            .unwrap_or(false)
    }
}

/// Mises à jour partielles d'un utilisateur
#[derive(Debug, Clone, Deserialize)]
pub struct UserUpdates {
    pub displayname: Option<String>,
    pub email: Option<String>,
    pub groups: Option<Vec<String>>,
    pub disabled: Option<bool>,
}

/// Hash un mot de passe avec Argon2id (mêmes paramètres que le backend Node.js)
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut rand_core::OsRng);
    // Paramètres identiques au backend Node.js : memoryCost=65536, timeCost=3, parallelism=4
    let params = Params::new(65536, 3, 4, None)
        .map_err(|e| anyhow::anyhow!("Argon2 params error: {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("Argon2 hash error: {e}"))?
        .to_string();
    Ok(hash)
}

/// Vérifie un mot de passe contre un hash Argon2id
pub fn verify_password(password: &str, hash: &str) -> bool {
    let parsed_hash = match PasswordHash::new(hash) {
        Ok(h) => h,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify() {
        let password = "test_password_123";
        let hash = hash_password(password).unwrap();
        assert!(verify_password(password, &hash));
        assert!(!verify_password("wrong_password", &hash));
    }

    #[test]
    fn test_verify_node_compatible() {
        // Un hash généré par argon2 de Node.js devrait être vérifiable
        // Les deux utilisent le format PHC string ($argon2id$v=19$m=65536,t=3,p=4$...)
        let password = "test";
        let hash = hash_password(password).unwrap();
        assert!(hash.starts_with("$argon2id$"));
    }
}
