//! Cloudflare DNS record management (A and AAAA records).
//! Extracted from hr-api/routes/ddns.rs for reuse by the agent registry.

use serde_json::json;
use tracing::{info, warn};

const CF_API_BASE: &str = "https://api.cloudflare.com/client/v4";

/// Create or update a Cloudflare AAAA record. Returns the record ID.
pub async fn upsert_aaaa_record(
    token: &str,
    zone_id: &str,
    record_name: &str,
    ipv6: &str,
    proxied: bool,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    // List existing AAAA records for this name
    let list_url = format!(
        "{}/zones/{}/dns_records?type=AAAA&name={}",
        CF_API_BASE, zone_id, record_name
    );

    let resp = client
        .get(&list_url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    check_cf_errors(&body)?;

    let records = body
        .get("result")
        .and_then(|r| r.as_array())
        .ok_or("Invalid response from Cloudflare")?;

    if let Some(record) = records.first() {
        // Update existing record
        let record_id = record
            .get("id")
            .and_then(|i| i.as_str())
            .ok_or("No record ID")?
            .to_string();

        let update_url = format!(
            "{}/zones/{}/dns_records/{}",
            CF_API_BASE, zone_id, record_id
        );

        let resp = client
            .put(&update_url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({
                "type": "AAAA",
                "name": record_name,
                "content": ipv6,
                "ttl": 120,
                "proxied": proxied
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Cloudflare update error: {}", body));
        }

        info!(record = record_name, ipv6, "Updated Cloudflare AAAA record");
        Ok(record_id)
    } else {
        // Create new record
        let create_url = format!("{}/zones/{}/dns_records", CF_API_BASE, zone_id);

        let resp = client
            .post(&create_url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({
                "type": "AAAA",
                "name": record_name,
                "content": ipv6,
                "ttl": 120,
                "proxied": proxied
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let resp_body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        check_cf_errors(&resp_body)?;

        let record_id = resp_body
            .get("result")
            .and_then(|r| r.get("id"))
            .and_then(|i| i.as_str())
            .ok_or("No record ID in create response")?
            .to_string();

        info!(record = record_name, ipv6, "Created Cloudflare AAAA record");
        Ok(record_id)
    }
}

/// Create or update a Cloudflare A record (IPv4). Returns the record ID.
pub async fn upsert_a_record(
    token: &str,
    zone_id: &str,
    record_name: &str,
    ipv4: &str,
    proxied: bool,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    // List existing A records for this name
    let list_url = format!(
        "{}/zones/{}/dns_records?type=A&name={}",
        CF_API_BASE, zone_id, record_name
    );

    let resp = client
        .get(&list_url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    check_cf_errors(&body)?;

    let records = body
        .get("result")
        .and_then(|r| r.as_array())
        .ok_or("Invalid response from Cloudflare")?;

    if let Some(record) = records.first() {
        // Update existing record
        let record_id = record
            .get("id")
            .and_then(|i| i.as_str())
            .ok_or("No record ID")?
            .to_string();

        let update_url = format!(
            "{}/zones/{}/dns_records/{}",
            CF_API_BASE, zone_id, record_id
        );

        let resp = client
            .put(&update_url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({
                "type": "A",
                "name": record_name,
                "content": ipv4,
                "ttl": 120,
                "proxied": proxied
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Cloudflare update error: {}", body));
        }

        info!(record = record_name, ipv4, "Updated Cloudflare A record");
        Ok(record_id)
    } else {
        // Create new record
        let create_url = format!("{}/zones/{}/dns_records", CF_API_BASE, zone_id);

        let resp = client
            .post(&create_url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({
                "type": "A",
                "name": record_name,
                "content": ipv4,
                "ttl": 120,
                "proxied": proxied
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let resp_body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        check_cf_errors(&resp_body)?;

        let record_id = resp_body
            .get("result")
            .and_then(|r| r.get("id"))
            .and_then(|i| i.as_str())
            .ok_or("No record ID in create response")?
            .to_string();

        info!(record = record_name, ipv4, "Created Cloudflare A record");
        Ok(record_id)
    }
}

/// Delete a Cloudflare DNS record by ID.
pub async fn delete_record(token: &str, zone_id: &str, record_id: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/zones/{}/dns_records/{}",
        CF_API_BASE, zone_id, record_id
    );

    let resp = client
        .delete(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        warn!(record_id, "Failed to delete Cloudflare record: {}", body);
        return Err(format!("Cloudflare delete error: {}", body));
    }

    info!(record_id, "Deleted Cloudflare DNS record");
    Ok(())
}

/// Delete a Cloudflare AAAA record by domain name.
/// Returns Ok(Some(record_id)) if deleted, Ok(None) if not found.
pub async fn delete_record_by_name(
    token: &str,
    zone_id: &str,
    record_name: &str,
) -> Result<Option<String>, String> {
    let client = reqwest::Client::new();

    // List existing AAAA records for this name
    let list_url = format!(
        "{}/zones/{}/dns_records?type=AAAA&name={}",
        CF_API_BASE, zone_id, record_name
    );

    let resp = client
        .get(&list_url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    check_cf_errors(&body)?;

    let records = body
        .get("result")
        .and_then(|r| r.as_array())
        .ok_or("Invalid response from Cloudflare")?;

    let Some(record) = records.first() else {
        return Ok(None);
    };

    let record_id = record
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or("No record ID")?
        .to_string();

    // Delete the record
    delete_record(token, zone_id, &record_id).await?;

    info!(record = record_name, "Deleted Cloudflare AAAA record by name");
    Ok(Some(record_id))
}

/// Delete a Cloudflare A record by domain name.
/// Returns Ok(Some(record_id)) if deleted, Ok(None) if not found.
pub async fn delete_a_record_by_name(
    token: &str,
    zone_id: &str,
    record_name: &str,
) -> Result<Option<String>, String> {
    let client = reqwest::Client::new();

    // List existing A records for this name
    let list_url = format!(
        "{}/zones/{}/dns_records?type=A&name={}",
        CF_API_BASE, zone_id, record_name
    );

    let resp = client
        .get(&list_url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    check_cf_errors(&body)?;

    let records = body
        .get("result")
        .and_then(|r| r.as_array())
        .ok_or("Invalid response from Cloudflare")?;

    let Some(record) = records.first() else {
        return Ok(None);
    };

    let record_id = record
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or("No record ID")?
        .to_string();

    // Delete the record
    delete_record(token, zone_id, &record_id).await?;

    info!(record = record_name, "Deleted Cloudflare A record by name");
    Ok(Some(record_id))
}

/// Get the content (IPv6 address) of an existing AAAA record.
pub async fn get_aaaa_record_content(
    token: &str,
    zone_id: &str,
    record_name: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/zones/{}/dns_records?type=AAAA&name={}",
        CF_API_BASE, zone_id, record_name
    );

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    check_cf_errors(&body)?;

    body.get("result")
        .and_then(|r| r.as_array())
        .and_then(|arr| arr.first())
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_str())
        .map(String::from)
        .ok_or_else(|| "Record not found".to_string())
}

/// Switch Cloudflare DNS to relay mode: create A record for VPS IPv4, remove AAAA record for on-prem.
/// Handles both the main wildcard (*.base_domain) and code wildcard (*.code.base_domain).
pub async fn switch_to_relay_dns(
    token: &str,
    zone_id: &str,
    base_domain: &str,
    vps_ipv4: &str,
) -> Result<(), String> {
    let main_wildcard = format!("*.{}", base_domain);
    let code_wildcard = format!("*.code.{}", base_domain);

    // 1. Upsert A record for *.base_domain → vps_ipv4 (proxied)
    upsert_a_record(token, zone_id, &main_wildcard, vps_ipv4, true).await?;
    info!(domain = %main_wildcard, ipv4 = vps_ipv4, "Relay DNS: set A record");

    // 2. Upsert A record for *.code.base_domain → vps_ipv4 (proxied)
    upsert_a_record(token, zone_id, &code_wildcard, vps_ipv4, true).await?;
    info!(domain = %code_wildcard, ipv4 = vps_ipv4, "Relay DNS: set A record");

    // 3. Delete AAAA record for *.base_domain (if exists)
    if let Some(id) = delete_record_by_name(token, zone_id, &main_wildcard).await? {
        info!(domain = %main_wildcard, record_id = %id, "Relay DNS: removed AAAA record");
    }

    // 4. Delete AAAA record for *.code.base_domain (if exists)
    if let Some(id) = delete_record_by_name(token, zone_id, &code_wildcard).await? {
        info!(domain = %code_wildcard, record_id = %id, "Relay DNS: removed AAAA record");
    }

    info!(base_domain, vps_ipv4, "Cloudflare DNS switched to relay mode");
    Ok(())
}

/// Switch Cloudflare DNS back to direct mode: create AAAA record for on-prem IPv6, remove A record for VPS.
/// Handles both the main wildcard and code wildcard.
pub async fn switch_to_direct_dns(
    token: &str,
    zone_id: &str,
    base_domain: &str,
    onprem_ipv6: &str,
) -> Result<(), String> {
    let main_wildcard = format!("*.{}", base_domain);
    let code_wildcard = format!("*.code.{}", base_domain);

    // 1. Upsert AAAA record for *.base_domain → onprem_ipv6 (proxied)
    upsert_aaaa_record(token, zone_id, &main_wildcard, onprem_ipv6, true).await?;
    info!(domain = %main_wildcard, ipv6 = onprem_ipv6, "Direct DNS: set AAAA record");

    // 2. Upsert AAAA record for *.code.base_domain → onprem_ipv6 (proxied)
    upsert_aaaa_record(token, zone_id, &code_wildcard, onprem_ipv6, true).await?;
    info!(domain = %code_wildcard, ipv6 = onprem_ipv6, "Direct DNS: set AAAA record");

    // 3. Delete A record for *.base_domain (if exists)
    if let Some(id) = delete_a_record_by_name(token, zone_id, &main_wildcard).await? {
        info!(domain = %main_wildcard, record_id = %id, "Direct DNS: removed A record");
    }

    // 4. Delete A record for *.code.base_domain (if exists)
    if let Some(id) = delete_a_record_by_name(token, zone_id, &code_wildcard).await? {
        info!(domain = %code_wildcard, record_id = %id, "Direct DNS: removed A record");
    }

    info!(base_domain, onprem_ipv6, "Cloudflare DNS switched to direct mode");
    Ok(())
}

fn check_cf_errors(body: &serde_json::Value) -> Result<(), String> {
    if let Some(false) = body.get("success").and_then(|s| s.as_bool()) {
        let errors = body
            .get("errors")
            .and_then(|e| e.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "Unknown error".to_string());
        return Err(format!("Cloudflare API: {}", errors));
    }
    Ok(())
}
