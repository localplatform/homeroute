//! Cloudflare DNS record management for ACME DNS-01 challenges

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

const CF_API_BASE: &str = "https://api.cloudflare.com/client/v4";

#[derive(Debug, Serialize)]
struct CreateRecordRequest<'a> {
    #[serde(rename = "type")]
    record_type: &'a str,
    name: &'a str,
    content: &'a str,
    ttl: u32,
}

#[derive(Debug, Deserialize)]
struct CloudflareResponse<T> {
    success: bool,
    result: Option<T>,
    errors: Option<Vec<CloudflareError>>,
}

#[derive(Debug, Deserialize)]
struct CloudflareError {
    code: u32,
    message: String,
}

#[derive(Debug, Deserialize)]
struct DnsRecord {
    id: String,
}

/// Create a TXT record for ACME DNS-01 challenge
pub async fn create_acme_challenge_record(
    token: &str,
    zone_id: &str,
    record_name: &str,
    challenge_value: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/zones/{}/dns_records", CF_API_BASE, zone_id);

    debug!(
        record_name,
        "Creating ACME challenge TXT record in Cloudflare"
    );

    let request = CreateRecordRequest {
        record_type: "TXT",
        name: record_name,
        content: challenge_value,
        ttl: 60,
    };

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let body: CloudflareResponse<DnsRecord> = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !body.success {
        let err_msg = body
            .errors
            .map(|errs| {
                errs.iter()
                    .map(|e| format!("[{}] {}", e.code, e.message))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| format!("HTTP {}", status));
        return Err(format!("Cloudflare API error: {}", err_msg));
    }

    let record_id = body
        .result
        .ok_or("No result in response")?
        .id;

    info!(record_name, record_id = %record_id, "Created ACME challenge TXT record");
    Ok(record_id)
}

/// Delete a TXT record after challenge completion
pub async fn delete_challenge_record(
    token: &str,
    zone_id: &str,
    record_id: &str,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = format!("{}/zones/{}/dns_records/{}", CF_API_BASE, zone_id, record_id);

    debug!(record_id, "Deleting ACME challenge TXT record");

    let resp = client
        .delete(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !resp.status().is_success() {
        warn!(record_id, status = %resp.status(), "Failed to delete challenge record");
        return Err(format!("Delete failed with status {}", resp.status()));
    }

    info!(record_id, "Deleted ACME challenge TXT record");
    Ok(())
}

/// List existing TXT records for cleanup
pub async fn list_acme_challenge_records(
    token: &str,
    zone_id: &str,
    name_prefix: &str,
) -> Result<Vec<(String, String)>, String> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/zones/{}/dns_records?type=TXT&name={}",
        CF_API_BASE, zone_id, name_prefix
    );

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    #[derive(Deserialize)]
    struct ListResult {
        id: String,
        name: String,
    }

    let body: CloudflareResponse<Vec<ListResult>> = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !body.success {
        return Err("Failed to list DNS records".into());
    }

    let records = body
        .result
        .unwrap_or_default()
        .into_iter()
        .map(|r| (r.id, r.name))
        .collect();

    Ok(records)
}
