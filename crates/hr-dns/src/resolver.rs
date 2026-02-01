use std::net::{Ipv4Addr, Ipv6Addr};
use tracing::{debug, warn};

use crate::SharedDnsState;
use crate::config::StaticRecord;
use crate::packet::{self, DnsQuery, RCODE_NOERROR, RCODE_NXDOMAIN, RCODE_SERVFAIL};
use crate::records::{DnsRecord, RecordType};

/// Result of DNS resolution
pub struct ResolveResult {
    pub records: Vec<DnsRecord>,
    pub rcode: u8,
    pub cached: bool,
    pub blocked: bool,
}

/// Resolve a DNS query through the resolution chain:
/// 1. DHCP lease hostnames (expand-hosts)
/// 2. Static records
/// 3. Wildcard local domain
/// 4. Adblock filter
/// 5. Cache
/// 6. Upstream forward
pub async fn resolve(query: &DnsQuery, state: &SharedDnsState) -> ResolveResult {
    if query.questions.is_empty() {
        return ResolveResult {
            records: vec![],
            rcode: RCODE_NOERROR,
            cached: false,
            blocked: false,
        };
    }

    let question = &query.questions[0];
    let name = &question.name;
    let qtype = question.qtype;

    let state_read = state.read().await;
    let config = &state_read.config;

    // 1. DHCP lease hostname lookup (expand-hosts)
    if config.expand_hosts && !config.local_domain.is_empty() {
        let hostname = if let Some(stripped) = name.strip_suffix(&format!(".{}", config.local_domain)) {
            Some(stripped.to_string())
        } else {
            None
        };

        if let Some(hostname) = hostname {
            if let Some(ip) = state_read.lease_store.read().await.find_ip_by_hostname(&hostname) {
                debug!("Resolved {} via DHCP lease -> {}", name, ip);
                return ResolveResult {
                    records: vec![DnsRecord::a(name, ip, 60)],
                    rcode: RCODE_NOERROR,
                    cached: false,
                    blocked: false,
                };
            }
        }
    }

    // 2. Static records
    for static_rec in &config.static_records {
        if static_rec.name.to_lowercase() == *name {
            let matching_type = match static_rec.record_type.to_uppercase().as_str() {
                "A" => RecordType::A,
                "AAAA" => RecordType::AAAA,
                "CNAME" => RecordType::CNAME,
                _ => continue,
            };

            if qtype == matching_type || qtype == RecordType::ANY {
                if let Some(record) = parse_static_record(name, static_rec, matching_type) {
                    debug!("Resolved {} via static record", name);
                    return ResolveResult {
                        records: vec![record],
                        rcode: RCODE_NOERROR,
                        cached: false,
                        blocked: false,
                    };
                }
            }
        }
    }

    // 3. Wildcard local domain (*.mynetwk.biz -> server IP)
    if !config.local_domain.is_empty() {
        let is_local = name.ends_with(&format!(".{}", config.local_domain))
            || *name == config.local_domain;

        if is_local {
            let records = match qtype {
                RecordType::A if !config.wildcard_ipv4.is_empty() => {
                    if let Ok(ip) = config.wildcard_ipv4.parse::<Ipv4Addr>() {
                        vec![DnsRecord::a(name, ip, 300)]
                    } else {
                        vec![]
                    }
                }
                RecordType::AAAA if !config.wildcard_ipv6.is_empty() => {
                    if let Ok(ip) = config.wildcard_ipv6.parse::<Ipv6Addr>() {
                        vec![DnsRecord::aaaa(name, ip, 300)]
                    } else {
                        vec![]
                    }
                }
                RecordType::ANY => {
                    let mut recs = vec![];
                    if let Ok(ip) = config.wildcard_ipv4.parse::<Ipv4Addr>() {
                        recs.push(DnsRecord::a(name, ip, 300));
                    }
                    if let Ok(ip) = config.wildcard_ipv6.parse::<Ipv6Addr>() {
                        recs.push(DnsRecord::aaaa(name, ip, 300));
                    }
                    recs
                }
                _ => vec![],
            };

            if !records.is_empty() {
                debug!("Resolved {} via wildcard local domain", name);
                return ResolveResult {
                    records,
                    rcode: RCODE_NOERROR,
                    cached: false,
                    blocked: false,
                };
            }

            // If it's a local domain but no matching record type, return empty (not NXDOMAIN)
            if matches!(qtype, RecordType::A | RecordType::AAAA) {
                return ResolveResult {
                    records: vec![],
                    rcode: RCODE_NOERROR,
                    cached: false,
                    blocked: false,
                };
            }
        }
    }

    // 4. Adblock filter
    if state_read.adblock_enabled && state_read.adblock.read().await.is_blocked(name) {
        debug!("Blocked {} via adblock", name);
        let records = match state_read.adblock_block_response.as_str() {
            "zero_ip" => match qtype {
                RecordType::A => vec![DnsRecord::a(name, Ipv4Addr::UNSPECIFIED, 300)],
                RecordType::AAAA => vec![DnsRecord::aaaa(name, Ipv6Addr::UNSPECIFIED, 300)],
                _ => vec![],
            },
            _ => {
                return ResolveResult {
                    records: vec![],
                    rcode: RCODE_NXDOMAIN,
                    cached: false,
                    blocked: true,
                };
            }
        };
        return ResolveResult {
            records,
            rcode: RCODE_NOERROR,
            cached: false,
            blocked: true,
        };
    }

    // 5. Cache lookup
    if let Some(cached_records) = state_read.dns_cache.get(name, qtype).await {
        debug!("Resolved {} via cache ({} records)", name, cached_records.len());
        return ResolveResult {
            records: cached_records,
            rcode: RCODE_NOERROR,
            cached: true,
            blocked: false,
        };
    }

    // 6. Upstream forward
    // Build a clean query to forward (only the first question)
    let _query_bytes = packet::build_response(query, &[], RCODE_NOERROR);
    // Actually we want to forward the original query bytes, but we reconstruct to ensure clean format.
    // We'll re-build a query packet:
    let forward_bytes = build_forward_query(query);

    match state_read.upstream.forward(&forward_bytes).await {
        Ok(response_bytes) => {
            match packet::parse_response_records(&response_bytes) {
                Ok((header, records)) => {
                    // Cache the result
                    if !records.is_empty() {
                        state_read.dns_cache.insert(name, qtype, &records).await;
                    }

                    debug!("Resolved {} via upstream ({} records)", name, records.len());
                    ResolveResult {
                        records,
                        rcode: header.rcode(),
                        cached: false,
                        blocked: false,
                    }
                }
                Err(e) => {
                    warn!("Failed to parse upstream response for {}: {}", name, e);
                    // Return raw response -- the upstream response is valid DNS, we just can't parse it
                    // Return SERVFAIL if we truly can't handle it
                    ResolveResult {
                        records: vec![],
                        rcode: RCODE_SERVFAIL,
                        cached: false,
                        blocked: false,
                    }
                }
            }
        }
        Err(e) => {
            warn!("Upstream forward failed for {}: {}", name, e);
            ResolveResult {
                records: vec![],
                rcode: RCODE_SERVFAIL,
                cached: false,
                blocked: false,
            }
        }
    }
}

fn build_forward_query(query: &DnsQuery) -> Vec<u8> {
    let mut buf = Vec::with_capacity(512);

    // Header
    buf.extend_from_slice(&query.header.id.to_be_bytes());
    // Flags: standard query with RD
    let flags: u16 = 0x0100; // RD=1
    buf.extend_from_slice(&flags.to_be_bytes());
    buf.extend_from_slice(&query.header.qd_count.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes()); // AN
    buf.extend_from_slice(&0u16.to_be_bytes()); // NS
    buf.extend_from_slice(&0u16.to_be_bytes()); // AR

    // Question section
    buf.extend_from_slice(&query.raw_question_bytes);

    buf
}

fn parse_static_record(name: &str, rec: &StaticRecord, rtype: RecordType) -> Option<DnsRecord> {
    match rtype {
        RecordType::A => {
            let ip: Ipv4Addr = rec.value.parse().ok()?;
            Some(DnsRecord::a(name, ip, rec.ttl))
        }
        RecordType::AAAA => {
            let ip: Ipv6Addr = rec.value.parse().ok()?;
            Some(DnsRecord::aaaa(name, ip, rec.ttl))
        }
        RecordType::CNAME => {
            Some(DnsRecord::cname(name, &rec.value, rec.ttl))
        }
        _ => None,
    }
}
