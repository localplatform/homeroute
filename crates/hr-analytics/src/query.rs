use anyhow::{bail, Result};
use serde_json::{json, Value};
use crate::store::AnalyticsStore;

/// Parse a human-readable time range string into seconds.
///
/// Supported formats: "1h", "6h", "24h", "7d", "30d", "90d", "365d"
fn parse_time_range(range: &str) -> Result<i64> {
    let range = range.trim();
    if range.is_empty() {
        return Ok(86400); // default 24h
    }

    let (num_str, unit) = if range.ends_with('d') {
        (&range[..range.len() - 1], 'd')
    } else if range.ends_with('h') {
        (&range[..range.len() - 1], 'h')
    } else {
        bail!("Invalid time range format '{}': must end with 'h' or 'd'", range);
    };

    let num: i64 = num_str
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid number in time range: '{}'", num_str))?;

    match unit {
        'h' => Ok(num * 3600),
        'd' => Ok(num * 86400),
        _ => unreachable!(),
    }
}

/// Compute the start timestamp (ISO 8601 UTC) for a given time range string.
fn start_timestamp(time_range: &str) -> Result<String> {
    let seconds = parse_time_range(time_range)?;
    let start = chrono::Utc::now() - chrono::Duration::seconds(seconds);
    Ok(start.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// Determine the best granularity format string for strftime based on the
/// requested granularity name.
fn granularity_fmt(granularity: &str) -> &'static str {
    match granularity {
        "minute" => "%Y-%m-%dT%H:%M:00Z",
        "hour" => "%Y-%m-%dT%H:00:00Z",
        "day" => "%Y-%m-%dT00:00:00Z",
        _ => "%Y-%m-%dT%H:00:00Z", // default to hour
    }
}

/// Get a high-level overview of traffic for the given time range.
///
/// Returns: total_requests, total_bytes, unique_devices, unique_endpoints
pub fn get_overview(store: &AnalyticsStore, time_range: &str) -> Result<Value> {
    let start = start_timestamp(time_range)?;

    store.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT
                 COUNT(*) AS total_requests,
                 COALESCE(SUM(response_bytes), 0) AS total_bytes,
                 COUNT(DISTINCT device_mac) AS unique_devices,
                 COUNT(DISTINCT endpoint) AS unique_endpoints
             FROM traffic_http
             WHERE timestamp >= ?1",
        )?;

        let row = stmt.query_row(rusqlite::params![start], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;

        Ok(json!({
            "total_requests": row.0,
            "total_bytes": row.1,
            "unique_devices": row.2,
            "unique_endpoints": row.3,
            "time_range": time_range,
        }))
    })
}

/// Get time-series data for a given metric and granularity.
///
/// `metric` is "requests" or "bytes".
/// `granularity` is "minute", "hour", or "day".
pub fn get_timeseries(
    store: &AnalyticsStore,
    metric: &str,
    granularity: &str,
    time_range: &str,
) -> Result<Value> {
    let start = start_timestamp(time_range)?;
    let fmt = granularity_fmt(granularity);

    let agg_expr = match metric {
        "requests" => "COUNT(*)",
        "bytes" => "COALESCE(SUM(response_bytes), 0)",
        _ => "COUNT(*)",
    };

    let sql = format!(
        "SELECT
             strftime('{}', timestamp) AS ts,
             {} AS value
         FROM traffic_http
         WHERE timestamp >= ?1
         GROUP BY ts
         ORDER BY ts",
        fmt, agg_expr
    );

    store.with_conn(|conn| {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![start], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
            ))
        })?;

        let mut data = Vec::new();
        for row in rows {
            let (ts, value) = row?;
            data.push(json!({
                "timestamp": ts,
                "value": value,
            }));
        }

        Ok(json!({
            "metric": metric,
            "granularity": granularity,
            "time_range": time_range,
            "data": data,
        }))
    })
}

/// Get top devices by total bytes transferred.
pub fn get_top_devices(store: &AnalyticsStore, time_range: &str, limit: i64) -> Result<Value> {
    let start = start_timestamp(time_range)?;

    store.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT
                 device_mac,
                 device_hostname,
                 COUNT(*) AS total_requests,
                 COALESCE(SUM(response_bytes), 0) AS total_bytes,
                 AVG(response_time_ms) AS avg_response_time
             FROM traffic_http
             WHERE timestamp >= ?1 AND device_mac IS NOT NULL
             GROUP BY device_mac
             ORDER BY total_bytes DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(rusqlite::params![start, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, f64>(4)?,
            ))
        })?;

        let mut devices = Vec::new();
        for row in rows {
            let (mac, hostname, requests, bytes, avg_time) = row?;
            devices.push(json!({
                "device_mac": mac,
                "device_hostname": hostname,
                "total_requests": requests,
                "total_bytes": bytes,
                "avg_response_time": avg_time,
            }));
        }

        Ok(json!({
            "time_range": time_range,
            "devices": devices,
        }))
    })
}

/// Get top endpoints by total requests.
pub fn get_top_endpoints(store: &AnalyticsStore, time_range: &str, limit: i64) -> Result<Value> {
    let start = start_timestamp(time_range)?;

    store.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT
                 endpoint,
                 COUNT(*) AS total_requests,
                 COALESCE(SUM(response_bytes), 0) AS total_bytes,
                 AVG(response_time_ms) AS avg_response_time,
                 COUNT(DISTINCT device_mac) AS unique_devices
             FROM traffic_http
             WHERE timestamp >= ?1 AND endpoint IS NOT NULL
             GROUP BY endpoint
             ORDER BY total_requests DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(rusqlite::params![start, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;

        let mut endpoints = Vec::new();
        for row in rows {
            let (endpoint, requests, bytes, avg_time, devices) = row?;
            endpoints.push(json!({
                "endpoint": endpoint,
                "total_requests": requests,
                "total_bytes": bytes,
                "avg_response_time": avg_time,
                "unique_devices": devices,
            }));
        }

        Ok(json!({
            "time_range": time_range,
            "endpoints": endpoints,
        }))
    })
}

/// Get top applications by total requests.
pub fn get_top_applications(
    store: &AnalyticsStore,
    time_range: &str,
    limit: i64,
) -> Result<Value> {
    let start = start_timestamp(time_range)?;

    store.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT
                 application,
                 environment,
                 COUNT(*) AS total_requests,
                 COALESCE(SUM(response_bytes), 0) AS total_bytes,
                 AVG(response_time_ms) AS avg_response_time,
                 COUNT(DISTINCT device_mac) AS unique_devices
             FROM traffic_http
             WHERE timestamp >= ?1 AND application IS NOT NULL
             GROUP BY application, environment
             ORDER BY total_requests DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(rusqlite::params![start, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;

        let mut applications = Vec::new();
        for row in rows {
            let (app, env, requests, bytes, avg_time, devices) = row?;
            applications.push(json!({
                "application": app,
                "environment": env,
                "total_requests": requests,
                "total_bytes": bytes,
                "avg_response_time": avg_time,
                "unique_devices": devices,
            }));
        }

        Ok(json!({
            "time_range": time_range,
            "applications": applications,
        }))
    })
}

/// Get detailed analytics for a specific device (by MAC address).
///
/// Returns the device's timeline, top endpoints, and top applications.
pub fn get_device_detail(store: &AnalyticsStore, mac: &str, time_range: &str) -> Result<Value> {
    let start = start_timestamp(time_range)?;

    store.with_conn(|conn| {
        // Timeline (hourly)
        let mut timeline_stmt = conn.prepare(
            "SELECT
                 strftime('%Y-%m-%dT%H:00:00Z', timestamp) AS ts,
                 COUNT(*) AS requests,
                 COALESCE(SUM(response_bytes), 0) AS bytes
             FROM traffic_http
             WHERE timestamp >= ?1 AND device_mac = ?2
             GROUP BY ts
             ORDER BY ts",
        )?;

        let timeline_rows = timeline_stmt.query_map(rusqlite::params![start, mac], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;

        let mut timeline = Vec::new();
        for row in timeline_rows {
            let (ts, requests, bytes) = row?;
            timeline.push(json!({
                "timestamp": ts,
                "requests": requests,
                "bytes": bytes,
            }));
        }

        // Top endpoints for this device
        let mut endpoints_stmt = conn.prepare(
            "SELECT
                 endpoint,
                 COUNT(*) AS total_requests,
                 COALESCE(SUM(response_bytes), 0) AS total_bytes
             FROM traffic_http
             WHERE timestamp >= ?1 AND device_mac = ?2 AND endpoint IS NOT NULL
             GROUP BY endpoint
             ORDER BY total_requests DESC
             LIMIT 20",
        )?;

        let endpoints_rows = endpoints_stmt.query_map(rusqlite::params![start, mac], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;

        let mut top_endpoints = Vec::new();
        for row in endpoints_rows {
            let (endpoint, requests, bytes) = row?;
            top_endpoints.push(json!({
                "endpoint": endpoint,
                "total_requests": requests,
                "total_bytes": bytes,
            }));
        }

        // Top applications for this device
        let mut apps_stmt = conn.prepare(
            "SELECT
                 application,
                 environment,
                 COUNT(*) AS total_requests,
                 COALESCE(SUM(response_bytes), 0) AS total_bytes
             FROM traffic_http
             WHERE timestamp >= ?1 AND device_mac = ?2 AND application IS NOT NULL
             GROUP BY application, environment
             ORDER BY total_requests DESC
             LIMIT 20",
        )?;

        let apps_rows = apps_stmt.query_map(rusqlite::params![start, mac], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;

        let mut top_applications = Vec::new();
        for row in apps_rows {
            let (app, env, requests, bytes) = row?;
            top_applications.push(json!({
                "application": app,
                "environment": env,
                "total_requests": requests,
                "total_bytes": bytes,
            }));
        }

        // Get device hostname (from most recent record)
        let hostname: Option<String> = conn
            .query_row(
                "SELECT device_hostname FROM traffic_http
                 WHERE device_mac = ?1 AND device_hostname IS NOT NULL
                 ORDER BY timestamp DESC LIMIT 1",
                rusqlite::params![mac],
                |row| row.get(0),
            )
            .ok();

        Ok(json!({
            "device_mac": mac,
            "device_hostname": hostname,
            "time_range": time_range,
            "timeline": timeline,
            "top_endpoints": top_endpoints,
            "top_applications": top_applications,
        }))
    })
}

/// Get top DNS domains by query count.
pub fn get_dns_top_domains(store: &AnalyticsStore, time_range: &str, limit: i64) -> Result<Value> {
    let start = start_timestamp(time_range)?;

    store.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT
                 domain,
                 category,
                 COUNT(*) AS total_queries,
                 SUM(CASE WHEN blocked = 1 THEN 1 ELSE 0 END) AS blocked_count,
                 SUM(CASE WHEN cached = 1 THEN 1 ELSE 0 END) AS cached_count,
                 AVG(response_time_ms) AS avg_response_time,
                 COUNT(DISTINCT client_mac) AS unique_clients
             FROM traffic_dns
             WHERE timestamp >= ?1
             GROUP BY domain
             ORDER BY total_queries DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(rusqlite::params![start, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?;

        let mut domains = Vec::new();
        for row in rows {
            let (domain, category, queries, blocked, cached, avg_time, clients) = row?;
            domains.push(json!({
                "domain": domain,
                "category": category,
                "total_queries": queries,
                "blocked_count": blocked,
                "cached_count": cached,
                "avg_response_time": avg_time,
                "unique_clients": clients,
            }));
        }

        Ok(json!({
            "time_range": time_range,
            "domains": domains,
        }))
    })
}

/// Get DNS queries grouped by category.
pub fn get_dns_by_category(store: &AnalyticsStore, time_range: &str, limit: i64) -> Result<Value> {
    let start = start_timestamp(time_range)?;

    store.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT
                 COALESCE(category, 'Other') AS cat,
                 COUNT(*) AS total_queries,
                 SUM(CASE WHEN blocked = 1 THEN 1 ELSE 0 END) AS blocked_count,
                 COUNT(DISTINCT domain) AS unique_domains,
                 COUNT(DISTINCT client_mac) AS unique_clients
             FROM traffic_dns
             WHERE timestamp >= ?1
             GROUP BY cat
             ORDER BY total_queries DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(rusqlite::params![start, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;

        let mut categories = Vec::new();
        for row in rows {
            let (category, queries, blocked, domains, clients) = row?;
            categories.push(json!({
                "category": category,
                "total_queries": queries,
                "blocked_count": blocked,
                "unique_domains": domains,
                "unique_clients": clients,
            }));
        }

        Ok(json!({
            "time_range": time_range,
            "categories": categories,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{AnalyticsStore, DnsRecord, HttpRecord};

    fn setup_store() -> AnalyticsStore {
        let store = AnalyticsStore::open(":memory:").unwrap();

        // Insert some test HTTP data
        let events: Vec<HttpRecord> = (0..5)
            .map(|i| HttpRecord {
                timestamp: format!("2025-06-15T10:{:02}:00Z", i),
                device_mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
                device_ip: Some("10.0.0.50".to_string()),
                device_hostname: Some("laptop".to_string()),
                endpoint: Some("example.com".to_string()),
                application: Some("web".to_string()),
                environment: Some("prod".to_string()),
                path: Some("/".to_string()),
                method: Some("GET".to_string()),
                status_code: Some(200),
                response_bytes: 1024,
                response_time_ms: 50,
            })
            .collect();
        store.insert_http_batch(&events).unwrap();

        // Insert some test DNS data
        for i in 0..3 {
            store
                .insert_dns(&DnsRecord {
                    timestamp: format!("2025-06-15T10:{:02}:00Z", i),
                    client_ip: Some("10.0.0.50".to_string()),
                    client_mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
                    client_hostname: Some("laptop".to_string()),
                    domain: "example.com".to_string(),
                    query_type: Some("A".to_string()),
                    category: Some("Other".to_string()),
                    blocked: false,
                    cached: i > 0,
                    response_time_ms: 10,
                })
                .unwrap();
        }

        store
    }

    #[test]
    fn test_parse_time_range() {
        assert_eq!(parse_time_range("1h").unwrap(), 3600);
        assert_eq!(parse_time_range("24h").unwrap(), 86400);
        assert_eq!(parse_time_range("7d").unwrap(), 604800);
        assert_eq!(parse_time_range("30d").unwrap(), 2592000);
        assert!(parse_time_range("invalid").is_err());
    }

    #[test]
    fn test_get_overview() {
        let store = setup_store();
        // Use a very wide time range to capture all test data
        let result = get_overview(&store, "365d").unwrap();
        assert_eq!(result["total_requests"], 5);
        assert_eq!(result["total_bytes"], 5120);
        assert_eq!(result["unique_devices"], 1);
        assert_eq!(result["unique_endpoints"], 1);
    }

    #[test]
    fn test_get_top_devices() {
        let store = setup_store();
        let result = get_top_devices(&store, "365d", 10).unwrap();
        let devices = result["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0]["device_mac"], "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn test_get_dns_top_domains() {
        let store = setup_store();
        let result = get_dns_top_domains(&store, "365d", 10).unwrap();
        let domains = result["domains"].as_array().unwrap();
        assert_eq!(domains.len(), 1);
        assert_eq!(domains[0]["domain"], "example.com");
        assert_eq!(domains[0]["total_queries"], 3);
    }

    #[test]
    fn test_get_dns_by_category() {
        let store = setup_store();
        let result = get_dns_by_category(&store, "365d", 10).unwrap();
        let categories = result["categories"].as_array().unwrap();
        assert!(!categories.is_empty());
    }
}
