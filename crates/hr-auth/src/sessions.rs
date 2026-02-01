use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

/// Durées de session
const SESSION_DURATION_MS: i64 = 60 * 60 * 1000; // 1 heure
const REMEMBER_ME_DURATION_MS: i64 = 30 * 24 * 60 * 60 * 1000; // 30 jours
const INACTIVITY_TIMEOUT_MS: i64 = 30 * 60 * 1000; // 30 minutes

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub last_activity: i64,
    pub remember_me: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub user_id: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub remember_me: bool,
}

/// Store de sessions SQLite (thread-safe via Mutex)
pub struct SessionStore {
    conn: Mutex<Connection>,
}

impl SessionStore {
    pub fn new(data_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join("auth.db");
        let conn = Connection::open(db_path)?;

        conn.pragma_update(None, "journal_mode", "WAL")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                ip_address TEXT,
                user_agent TEXT,
                last_activity INTEGER NOT NULL,
                remember_me INTEGER DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions(expires_at);",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Crée une nouvelle session
    pub fn create(
        &self,
        user_id: &str,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
        remember_me: bool,
    ) -> anyhow::Result<(String, i64)> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let now = now_ms();
        let duration = if remember_me {
            REMEMBER_ME_DURATION_MS
        } else {
            SESSION_DURATION_MS
        };
        let expires_at = now + duration;

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, user_id, created_at, expires_at, ip_address, user_agent, last_activity, remember_me)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session_id,
                user_id,
                now,
                expires_at,
                ip_address,
                user_agent,
                now,
                remember_me as i32,
            ],
        )?;

        Ok((session_id, expires_at))
    }

    /// Récupère une session par ID (vérifie expiration et inactivité)
    pub fn get(&self, session_id: &str) -> anyhow::Result<Option<Session>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, created_at, expires_at, ip_address, user_agent, last_activity, remember_me
             FROM sessions WHERE id = ?1",
        )?;

        let session = stmt
            .query_row(params![session_id], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    created_at: row.get(2)?,
                    expires_at: row.get(3)?,
                    ip_address: row.get(4)?,
                    user_agent: row.get(5)?,
                    last_activity: row.get(6)?,
                    remember_me: row.get::<_, i32>(7)? == 1,
                })
            })
            .optional()?;

        let Some(session) = session else {
            return Ok(None);
        };

        let now = now_ms();

        // Vérifier expiration
        if session.expires_at < now {
            drop(stmt);
            conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
            return Ok(None);
        }

        // Vérifier inactivité (sauf remember_me)
        if !session.remember_me && (now - session.last_activity) > INACTIVITY_TIMEOUT_MS {
            drop(stmt);
            conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
            return Ok(None);
        }

        Ok(Some(session))
    }

    /// Valide une session et met à jour l'activité
    pub fn validate(&self, session_id: &str) -> anyhow::Result<Option<SessionInfo>> {
        let session = self.get(session_id)?;

        let Some(session) = session else {
            return Ok(None);
        };

        // Mettre à jour last_activity
        self.update_activity(session_id)?;

        Ok(Some(SessionInfo {
            user_id: session.user_id,
            created_at: session.created_at,
            expires_at: session.expires_at,
            ip_address: session.ip_address,
            user_agent: session.user_agent,
            remember_me: session.remember_me,
        }))
    }

    /// Met à jour le timestamp d'activité
    pub fn update_activity(&self, session_id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET last_activity = ?1 WHERE id = ?2",
            params![now_ms(), session_id],
        )?;
        Ok(())
    }

    /// Supprime une session (logout)
    pub fn delete(&self, session_id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
        Ok(())
    }

    /// Supprime toutes les sessions d'un utilisateur
    pub fn delete_by_user(&self, user_id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM sessions WHERE user_id = ?1",
            params![user_id],
        )?;
        Ok(())
    }

    /// Récupère toutes les sessions d'un utilisateur
    pub fn get_by_user(&self, user_id: &str) -> anyhow::Result<Vec<Session>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, created_at, expires_at, ip_address, user_agent, last_activity, remember_me
             FROM sessions WHERE user_id = ?1",
        )?;

        let sessions = stmt
            .query_map(params![user_id], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    created_at: row.get(2)?,
                    expires_at: row.get(3)?,
                    ip_address: row.get(4)?,
                    user_agent: row.get(5)?,
                    last_activity: row.get(6)?,
                    remember_me: row.get::<_, i32>(7)? == 1,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(sessions)
    }

    /// Nettoie les sessions expirées
    pub fn cleanup_expired(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM sessions WHERE expires_at < ?1",
            params![now_ms()],
        )?;
        Ok(())
    }
}

/// Trait d'extension pour rusqlite optionnel
trait OptionalExt<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
