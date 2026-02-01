use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tracing::info;

/// SQLite-based analytics storage.
pub struct AnalyticsStore {
    conn: Arc<Mutex<Connection>>,
}

// --- Record types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRecord {
    pub timestamp: String,
    pub device_mac: Option<String>,
    pub device_ip: Option<String>,
    pub device_hostname: Option<String>,
    pub endpoint: Option<String>,
    pub application: Option<String>,
    pub environment: Option<String>,
    pub path: Option<String>,
    pub method: Option<String>,
    pub status_code: Option<i32>,
    pub response_bytes: i64,
    pub response_time_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsRecord {
    pub timestamp: String,
    pub client_ip: Option<String>,
    pub client_mac: Option<String>,
    pub client_hostname: Option<String>,
    pub domain: String,
    pub query_type: Option<String>,
    pub category: Option<String>,
    pub blocked: bool,
    pub cached: bool,
    pub response_time_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRecord {
    pub timestamp: String,
    pub chain: String,
    pub bytes_per_second: f64,
    pub packets_per_second: f64,
    pub total_bytes: i64,
    pub total_packets: i64,
}

impl AnalyticsStore {
    /// Open (or create) the SQLite database at `path`, enable WAL mode,
    /// and create all required tables and indexes.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open analytics DB at {}", path))?;

        // WAL mode for better concurrent read/write performance
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        // Create tables
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS traffic_http (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                device_mac TEXT,
                device_ip TEXT,
                device_hostname TEXT,
                endpoint TEXT,
                application TEXT,
                environment TEXT,
                path TEXT,
                method TEXT,
                status_code INTEGER,
                response_bytes INTEGER DEFAULT 0,
                response_time_ms INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS traffic_dns (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                client_ip TEXT,
                client_mac TEXT,
                client_hostname TEXT,
                domain TEXT NOT NULL,
                query_type TEXT,
                category TEXT,
                blocked INTEGER DEFAULT 0,
                cached INTEGER DEFAULT 0,
                response_time_ms INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS traffic_network (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                chain TEXT,
                bytes_per_second REAL,
                packets_per_second REAL,
                total_bytes INTEGER,
                total_packets INTEGER
            );

            CREATE TABLE IF NOT EXISTS traffic_hourly (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                device_mac TEXT,
                endpoint TEXT,
                application TEXT,
                environment TEXT,
                total_requests INTEGER,
                total_bytes INTEGER,
                avg_response_time REAL,
                event_count INTEGER,
                UNIQUE(timestamp, device_mac, endpoint, application)
            );

            CREATE TABLE IF NOT EXISTS traffic_daily (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                device_mac TEXT,
                endpoint TEXT,
                application TEXT,
                environment TEXT,
                total_requests INTEGER,
                total_bytes INTEGER,
                avg_response_time REAL,
                event_count INTEGER,
                UNIQUE(timestamp, device_mac, endpoint, application)
            );

            -- Indexes on timestamp
            CREATE INDEX IF NOT EXISTS idx_http_timestamp ON traffic_http(timestamp);
            CREATE INDEX IF NOT EXISTS idx_dns_timestamp ON traffic_dns(timestamp);
            CREATE INDEX IF NOT EXISTS idx_network_timestamp ON traffic_network(timestamp);
            CREATE INDEX IF NOT EXISTS idx_hourly_timestamp ON traffic_hourly(timestamp);
            CREATE INDEX IF NOT EXISTS idx_daily_timestamp ON traffic_daily(timestamp);

            -- Indexes on device_mac
            CREATE INDEX IF NOT EXISTS idx_http_device_mac ON traffic_http(device_mac);
            CREATE INDEX IF NOT EXISTS idx_dns_client_mac ON traffic_dns(client_mac);

            -- Index on endpoint
            CREATE INDEX IF NOT EXISTS idx_http_endpoint ON traffic_http(endpoint);

            -- Index on domain
            CREATE INDEX IF NOT EXISTS idx_dns_domain ON traffic_dns(domain);
            ",
        )
        .context("Failed to create analytics tables")?;

        info!("Analytics store opened at {}", path);

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Insert a single HTTP traffic record.
    pub fn insert_http(&self, event: &HttpRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO traffic_http (timestamp, device_mac, device_ip, device_hostname,
             endpoint, application, environment, path, method, status_code,
             response_bytes, response_time_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                event.timestamp,
                event.device_mac,
                event.device_ip,
                event.device_hostname,
                event.endpoint,
                event.application,
                event.environment,
                event.path,
                event.method,
                event.status_code,
                event.response_bytes,
                event.response_time_ms,
            ],
        )?;
        Ok(())
    }

    /// Insert a batch of HTTP traffic records in a single transaction.
    pub fn insert_http_batch(&self, events: &[HttpRecord]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO traffic_http (timestamp, device_mac, device_ip, device_hostname,
                 endpoint, application, environment, path, method, status_code,
                 response_bytes, response_time_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )?;
            for event in events {
                stmt.execute(rusqlite::params![
                    event.timestamp,
                    event.device_mac,
                    event.device_ip,
                    event.device_hostname,
                    event.endpoint,
                    event.application,
                    event.environment,
                    event.path,
                    event.method,
                    event.status_code,
                    event.response_bytes,
                    event.response_time_ms,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Insert a single DNS traffic record.
    pub fn insert_dns(&self, event: &DnsRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO traffic_dns (timestamp, client_ip, client_mac, client_hostname,
             domain, query_type, category, blocked, cached, response_time_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                event.timestamp,
                event.client_ip,
                event.client_mac,
                event.client_hostname,
                event.domain,
                event.query_type,
                event.category,
                event.blocked as i32,
                event.cached as i32,
                event.response_time_ms,
            ],
        )?;
        Ok(())
    }

    /// Insert a batch of DNS traffic records in a single transaction.
    pub fn insert_dns_batch(&self, events: &[DnsRecord]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO traffic_dns (timestamp, client_ip, client_mac, client_hostname,
                 domain, query_type, category, blocked, cached, response_time_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            for event in events {
                stmt.execute(rusqlite::params![
                    event.timestamp,
                    event.client_ip,
                    event.client_mac,
                    event.client_hostname,
                    event.domain,
                    event.query_type,
                    event.category,
                    event.blocked as i32,
                    event.cached as i32,
                    event.response_time_ms,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Insert a single network traffic record.
    pub fn insert_network(&self, event: &NetworkRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO traffic_network (timestamp, chain, bytes_per_second,
             packets_per_second, total_bytes, total_packets)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                event.timestamp,
                event.chain,
                event.bytes_per_second,
                event.packets_per_second,
                event.total_bytes,
                event.total_packets,
            ],
        )?;
        Ok(())
    }

    /// Aggregate traffic_http from the last hour into traffic_hourly.
    /// Uses INSERT OR REPLACE to upsert aggregated rows grouped by
    /// truncated hour, device_mac, endpoint, and application.
    pub fn aggregate_to_hourly(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now();
        let one_hour_ago = (now - chrono::Duration::hours(1))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        conn.execute(
            "INSERT OR REPLACE INTO traffic_hourly
             (timestamp, device_mac, endpoint, application, environment,
              total_requests, total_bytes, avg_response_time, event_count)
             SELECT
                 strftime('%Y-%m-%dT%H:00:00Z', timestamp) AS ts_hour,
                 COALESCE(device_mac, ''),
                 COALESCE(endpoint, ''),
                 COALESCE(application, ''),
                 environment,
                 COUNT(*) AS total_requests,
                 SUM(response_bytes) AS total_bytes,
                 AVG(response_time_ms) AS avg_response_time,
                 COUNT(*) AS event_count
             FROM traffic_http
             WHERE timestamp >= ?1
             GROUP BY ts_hour, device_mac, endpoint, application",
            rusqlite::params![one_hour_ago],
        )?;

        info!("Hourly aggregation completed");
        Ok(())
    }

    /// Aggregate traffic_hourly from the last 2 days into traffic_daily.
    pub fn aggregate_to_daily(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now();
        let two_days_ago = (now - chrono::Duration::days(2))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        conn.execute(
            "INSERT OR REPLACE INTO traffic_daily
             (timestamp, device_mac, endpoint, application, environment,
              total_requests, total_bytes, avg_response_time, event_count)
             SELECT
                 strftime('%Y-%m-%dT00:00:00Z', timestamp) AS ts_day,
                 device_mac,
                 endpoint,
                 application,
                 environment,
                 SUM(total_requests) AS total_requests,
                 SUM(total_bytes) AS total_bytes,
                 AVG(avg_response_time) AS avg_response_time,
                 SUM(event_count) AS event_count
             FROM traffic_hourly
             WHERE timestamp >= ?1
             GROUP BY ts_day, device_mac, endpoint, application",
            rusqlite::params![two_days_ago],
        )?;

        info!("Daily aggregation completed");
        Ok(())
    }

    /// Delete old data to keep the database size manageable.
    /// - traffic_http: older than 30 days
    /// - traffic_dns: older than 30 days
    /// - traffic_network: older than 30 days
    /// - traffic_hourly: older than 90 days
    /// - traffic_daily: older than 365 days
    pub fn cleanup_old_data(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now();

        let cutoff_30d = (now - chrono::Duration::days(30))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let cutoff_90d = (now - chrono::Duration::days(90))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let cutoff_365d = (now - chrono::Duration::days(365))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        let deleted_http = conn.execute(
            "DELETE FROM traffic_http WHERE timestamp < ?1",
            rusqlite::params![cutoff_30d],
        )?;
        let deleted_dns = conn.execute(
            "DELETE FROM traffic_dns WHERE timestamp < ?1",
            rusqlite::params![cutoff_30d],
        )?;
        let deleted_network = conn.execute(
            "DELETE FROM traffic_network WHERE timestamp < ?1",
            rusqlite::params![cutoff_30d],
        )?;
        let deleted_hourly = conn.execute(
            "DELETE FROM traffic_hourly WHERE timestamp < ?1",
            rusqlite::params![cutoff_90d],
        )?;
        let deleted_daily = conn.execute(
            "DELETE FROM traffic_daily WHERE timestamp < ?1",
            rusqlite::params![cutoff_365d],
        )?;

        info!(
            "Cleanup: deleted {} http, {} dns, {} network, {} hourly, {} daily records",
            deleted_http, deleted_dns, deleted_network, deleted_hourly, deleted_daily
        );

        Ok(())
    }

    /// Execute a read-only query and return the connection lock.
    /// Used by the query module for flexible querying.
    pub(crate) fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self.conn.lock().unwrap();
        f(&conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_and_insert() {
        let store = AnalyticsStore::open(":memory:").unwrap();

        let record = HttpRecord {
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            device_mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
            device_ip: Some("10.0.0.50".to_string()),
            device_hostname: Some("testhost".to_string()),
            endpoint: Some("example.com".to_string()),
            application: Some("web".to_string()),
            environment: None,
            path: Some("/index.html".to_string()),
            method: Some("GET".to_string()),
            status_code: Some(200),
            response_bytes: 1024,
            response_time_ms: 50,
        };

        store.insert_http(&record).unwrap();

        let dns = DnsRecord {
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            client_ip: Some("10.0.0.50".to_string()),
            client_mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
            client_hostname: Some("testhost".to_string()),
            domain: "example.com".to_string(),
            query_type: Some("A".to_string()),
            category: Some("Other".to_string()),
            blocked: false,
            cached: false,
            response_time_ms: 10,
        };

        store.insert_dns(&dns).unwrap();

        let net = NetworkRecord {
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            chain: "FORWARD".to_string(),
            bytes_per_second: 1000.0,
            packets_per_second: 100.0,
            total_bytes: 100000,
            total_packets: 10000,
        };

        store.insert_network(&net).unwrap();
    }

    #[test]
    fn test_batch_insert() {
        let store = AnalyticsStore::open(":memory:").unwrap();

        let events: Vec<HttpRecord> = (0..10)
            .map(|i| HttpRecord {
                timestamp: format!("2025-01-01T00:{:02}:00Z", i),
                device_mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
                device_ip: Some("10.0.0.50".to_string()),
                device_hostname: None,
                endpoint: Some("example.com".to_string()),
                application: None,
                environment: None,
                path: Some("/".to_string()),
                method: Some("GET".to_string()),
                status_code: Some(200),
                response_bytes: 512,
                response_time_ms: 20,
            })
            .collect();

        store.insert_http_batch(&events).unwrap();
    }
}
