use std::net::Ipv4Addr;
use std::sync::Arc;

use tokio::sync::broadcast;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use hr_common::events::{DnsTrafficEvent, HttpTrafficEvent};
use hr_dhcp::LeaseStore;

use crate::store::{AnalyticsStore, DnsRecord, HttpRecord};

/// Domain categories for DNS classification.
const DOMAIN_CATEGORIES: &[(&str, &str)] = &[
    ("youtube.com", "Video Streaming"),
    ("googlevideo.com", "Video Streaming"),
    ("netflix.com", "Video Streaming"),
    ("twitch.tv", "Video Streaming"),
    ("google.com", "Search & Web"),
    ("googleapis.com", "Cloud Services"),
    ("cloudflare.com", "CDN"),
    ("facebook.com", "Social Media"),
    ("instagram.com", "Social Media"),
    ("twitter.com", "Social Media"),
    ("microsoft.com", "Cloud Services"),
    ("windows.com", "Operating System"),
    ("apple.com", "Operating System"),
    ("icloud.com", "Cloud Services"),
    ("amazon.com", "E-commerce"),
    ("amazonaws.com", "Cloud Services"),
    ("spotify.com", "Music Streaming"),
    ("github.com", "Development"),
    ("gitlab.com", "Development"),
    ("mynetwk.biz", "Local Services"),
];

/// Run HTTP traffic capture from a broadcast channel.
///
/// Events are batched (up to 100 or every 5 seconds) then inserted into SQLite.
pub async fn run_http_capture(
    store: Arc<AnalyticsStore>,
    mut rx: broadcast::Receiver<HttpTrafficEvent>,
    dhcp_leases: Arc<RwLock<LeaseStore>>,
) {
    info!("HTTP traffic capture started");
    let mut batch: Vec<HttpRecord> = Vec::new();
    let batch_interval = tokio::time::Duration::from_secs(5);
    let max_batch = 100;

    loop {
        let deadline = tokio::time::sleep(batch_interval);
        tokio::pin!(deadline);

        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        // Enrich: look up device by IP in DHCP leases
                        let (device_mac, device_hostname) = {
                            let leases = dhcp_leases.read().await;
                            if let Ok(ip) = event.client_ip.parse::<Ipv4Addr>() {
                                if let Some(lease) = leases.get_lease(ip) {
                                    (
                                        Some(lease.mac.clone()),
                                        lease.hostname.clone(),
                                    )
                                } else {
                                    (None, None)
                                }
                            } else {
                                (None, None)
                            }
                        };

                        batch.push(HttpRecord {
                            timestamp: event.timestamp,
                            device_mac,
                            device_ip: Some(event.client_ip),
                            device_hostname,
                            endpoint: Some(event.host),
                            application: None,
                            environment: None,
                            path: Some(event.path),
                            method: Some(event.method),
                            status_code: Some(event.status as i32),
                            response_bytes: event.response_bytes as i64,
                            response_time_ms: event.duration_ms as i64,
                        });

                        if batch.len() >= max_batch {
                            flush_http_batch(&store, &mut batch);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("HTTP capture lagged, missed {} events", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("HTTP traffic channel closed, stopping capture");
                        break;
                    }
                }
            }
            _ = &mut deadline => {
                if !batch.is_empty() {
                    flush_http_batch(&store, &mut batch);
                }
            }
        }
    }

    // Flush remaining events
    if !batch.is_empty() {
        flush_http_batch(&store, &mut batch);
    }
}

fn flush_http_batch(store: &AnalyticsStore, batch: &mut Vec<HttpRecord>) {
    let events: Vec<HttpRecord> = batch.drain(..).collect();
    if let Err(e) = store.insert_http_batch(&events) {
        warn!("Failed to insert HTTP batch: {}", e);
    } else {
        debug!("Inserted {} HTTP events", events.len());
    }
}

/// Run DNS traffic capture from a broadcast channel.
///
/// Events are batched (up to 100 or every 5 seconds) then inserted into SQLite.
pub async fn run_dns_capture(
    store: Arc<AnalyticsStore>,
    mut rx: broadcast::Receiver<DnsTrafficEvent>,
    dhcp_leases: Arc<RwLock<LeaseStore>>,
) {
    info!("DNS traffic capture started");
    let mut batch: Vec<DnsRecord> = Vec::new();
    let batch_interval = tokio::time::Duration::from_secs(5);
    let max_batch = 100;

    loop {
        let deadline = tokio::time::sleep(batch_interval);
        tokio::pin!(deadline);

        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        // Skip reverse DNS lookups
                        if event.domain.contains(".in-addr.arpa") || event.domain.contains(".ip6.arpa") {
                            continue;
                        }

                        let (client_mac, client_hostname) = {
                            let leases = dhcp_leases.read().await;
                            if let Ok(ip) = event.client_ip.parse::<Ipv4Addr>() {
                                if let Some(lease) = leases.get_lease(ip) {
                                    (
                                        Some(lease.mac.clone()),
                                        lease.hostname.clone(),
                                    )
                                } else {
                                    (None, None)
                                }
                            } else {
                                (None, None)
                            }
                        };

                        let category = categorize_domain(&event.domain);

                        batch.push(DnsRecord {
                            timestamp: event.timestamp,
                            client_ip: Some(event.client_ip),
                            client_mac,
                            client_hostname,
                            domain: extract_base_domain(&event.domain),
                            query_type: Some(event.query_type),
                            category: Some(category.to_string()),
                            blocked: event.blocked,
                            cached: event.cached,
                            response_time_ms: event.response_time_ms as i64,
                        });

                        if batch.len() >= max_batch {
                            flush_dns_batch(&store, &mut batch);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("DNS capture lagged, missed {} events", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("DNS traffic channel closed, stopping capture");
                        break;
                    }
                }
            }
            _ = &mut deadline => {
                if !batch.is_empty() {
                    flush_dns_batch(&store, &mut batch);
                }
            }
        }
    }

    // Flush remaining events
    if !batch.is_empty() {
        flush_dns_batch(&store, &mut batch);
    }
}

fn flush_dns_batch(store: &AnalyticsStore, batch: &mut Vec<DnsRecord>) {
    let events: Vec<DnsRecord> = batch.drain(..).collect();
    if let Err(e) = store.insert_dns_batch(&events) {
        warn!("Failed to insert DNS batch: {}", e);
    } else {
        debug!("Inserted {} DNS events", events.len());
    }
}

/// Match a domain against known categories.
fn categorize_domain(domain: &str) -> &'static str {
    for &(pattern, category) in DOMAIN_CATEGORIES {
        if domain.contains(pattern) {
            return category;
        }
    }
    "Other"
}

/// Extract the base domain (last two labels) from a full domain name.
fn extract_base_domain(domain: &str) -> String {
    let parts: Vec<&str> = domain.split('.').collect();
    if parts.len() >= 2 {
        parts[parts.len() - 2..].join(".")
    } else {
        domain.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_categorize_domain() {
        assert_eq!(categorize_domain("www.youtube.com"), "Video Streaming");
        assert_eq!(categorize_domain("api.github.com"), "Development");
        assert_eq!(categorize_domain("unknown.xyz"), "Other");
    }

    #[test]
    fn test_extract_base_domain() {
        assert_eq!(extract_base_domain("www.youtube.com"), "youtube.com");
        assert_eq!(extract_base_domain("sub.deep.example.org"), "example.org");
        assert_eq!(extract_base_domain("localhost"), "localhost");
    }
}
