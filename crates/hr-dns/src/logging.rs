use chrono::Utc;
use serde::Serialize;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing::{error, debug};

#[derive(Serialize)]
struct QueryLogEntry {
    ts: String,
    #[serde(rename = "type")]
    query_type: String,
    domain: String,
    from: String,
    blocked: bool,
    cached: bool,
    ms: u64,
}

/// Async query logger using a background writer (same pattern as rust-proxy).
pub struct QueryLogger {
    sender: mpsc::UnboundedSender<String>,
}

impl QueryLogger {
    /// Create a new query logger writing to the given path.
    /// Spawns a background task for non-blocking file I/O.
    pub fn new(path: &str) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<String>();
        let path = PathBuf::from(path);

        tokio::spawn(async move {
            use tokio::fs::OpenOptions;
            use tokio::io::AsyncWriteExt;

            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }

            let mut file = match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
            {
                Ok(f) => f,
                Err(e) => {
                    error!("Failed to open query log file {}: {}", path.display(), e);
                    return;
                }
            };

            while let Some(line) = receiver.recv().await {
                if let Err(e) = file.write_all(line.as_bytes()).await {
                    error!("Failed to write to query log: {}", e);
                }
            }
        });

        Self { sender }
    }

    pub fn log(
        &self,
        domain: &str,
        query_type: &str,
        source_ip: &str,
        blocked: bool,
        cached: bool,
        elapsed_ms: u64,
    ) {
        let entry = QueryLogEntry {
            ts: Utc::now().to_rfc3339(),
            query_type: query_type.to_string(),
            domain: domain.to_string(),
            from: source_ip.to_string(),
            blocked,
            cached,
            ms: elapsed_ms,
        };

        match serde_json::to_string(&entry) {
            Ok(json) => {
                let line = format!("{}\n", json);
                if self.sender.send(line).is_err() {
                    debug!("Query log channel closed");
                }
            }
            Err(e) => {
                debug!("Failed to serialize query log entry: {}", e);
            }
        }
    }
}
