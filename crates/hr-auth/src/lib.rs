pub mod forward_auth;
pub mod middleware;
pub mod sessions;
pub mod users;

use crate::sessions::SessionStore;
use crate::users::UserStore;
use std::path::Path;
use std::sync::Arc;

/// Service d'authentification unifié
///
/// Encapsule le store de sessions (SQLite) et le store d'utilisateurs (YAML).
/// Thread-safe via Arc, utilisable depuis le proxy (forward-auth) et l'API.
pub struct AuthService {
    pub sessions: SessionStore,
    pub users: UserStore,
    pub base_domain: String,
}

impl AuthService {
    /// Crée et initialise le service d'authentification
    pub fn new(data_dir: &Path, base_domain: &str) -> anyhow::Result<Arc<Self>> {
        let sessions = SessionStore::new(data_dir)?;
        let users = UserStore::new(data_dir);

        Ok(Arc::new(Self {
            sessions,
            users,
            base_domain: base_domain.to_string(),
        }))
    }

    /// Démarre le nettoyage périodique des sessions expirées
    pub fn start_cleanup_task(self: &Arc<Self>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                if let Err(e) = this.sessions.cleanup_expired() {
                    tracing::warn!("Session cleanup error: {}", e);
                }
            }
        });
    }
}
