use chrono::Utc;
use serde::Serialize;
use std::path::PathBuf;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tracing::{error, info};

#[derive(Debug, Serialize)]
pub struct AccessLogEntry {
    pub timestamp: String,
    pub client_ip: String,
    pub host: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration_ms: u64,
    pub user_agent: String,
}

/// Async access logger that writes JSON lines via a channel
#[derive(Clone)]
pub struct AccessLogger {
    sender: mpsc::UnboundedSender<AccessLogEntry>,
}

impl AccessLogger {
    /// Start the access logger. Spawns a background task that writes to the log file.
    pub fn start(log_path: PathBuf) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<AccessLogEntry>();

        tokio::spawn(async move {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .await;

            let mut file = match file {
                Ok(f) => f,
                Err(e) => {
                    error!("Failed to open access log file {:?}: {}", log_path, e);
                    return;
                }
            };

            info!("Access logging to {:?}", log_path);

            while let Some(entry) = receiver.recv().await {
                match serde_json::to_string(&entry) {
                    Ok(json) => {
                        let line = format!("{}\n", json);
                        if let Err(e) = file.write_all(line.as_bytes()).await {
                            error!("Failed to write access log: {}", e);
                        }
                    }
                    Err(e) => {
                        error!("Failed to serialize access log entry: {}", e);
                    }
                }
            }
        });

        Self { sender }
    }

    /// Log an access entry (non-blocking)
    pub fn log(&self, entry: AccessLogEntry) {
        let _ = self.sender.send(entry);
    }
}

/// Optional access logger wrapper
#[derive(Clone)]
pub struct OptionalAccessLogger {
    inner: Option<AccessLogger>,
}

impl OptionalAccessLogger {
    pub fn new(log_path: Option<String>) -> Self {
        let inner = log_path.map(|p| AccessLogger::start(PathBuf::from(p)));
        Self { inner }
    }

    pub fn none() -> Self {
        Self { inner: None }
    }

    pub fn log(&self, entry: AccessLogEntry) {
        if let Some(logger) = &self.inner {
            logger.log(entry);
        }
    }
}

/// Create a timestamp string for the current time
pub fn now_timestamp() -> String {
    Utc::now().to_rfc3339()
}
