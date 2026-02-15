use std::net::{Ipv4Addr, Ipv6Addr};
use tracing::{debug, warn};

use crate::SharedDnsState;
use crate::config::StaticRecord;
use crate::packet::{self, DnsQuery, RCODE_NOERROR, RCODE_NXDOMAIN, RCODE_SERVFAIL};
use crate::records::{DnsRecord, RData, RecordType};

/// Result of DNS resolution
pub struct ResolveResult {
    pub records: Vec<DnsRecord>,
    pub rcode: u8,
    pub cached: bool,
    pub blocked: bool,
}

/// Resolve a DNS query through the resolution chain:
/// 1. DHCP lease hostnames (expand-hosts)
/// 2. Static records (exact match, then wildcard)
/// 3. Wildcard local domain (fallback for unknown hosts)
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
                if qtype == RecordType::A || qtype == RecordType::ANY {
                    return ResolveResult {
                        records: vec![DnsRecord::a(name, ip, 60)],
                        rcode: RCODE_NOERROR,
                        cached: false,
                        blocked: false,
                    };
                }
                // Hostname exists in DHCP leases but only has IPv4 — return NODATA
                // (empty answer, NOERROR) to prevent wildcard fallback returning wrong IP
                return ResolveResult {
                    records: vec![],
                    rcode: RCODE_NOERROR,
                    cached: false,
                    blocked: false,
                };
            }
        }
    }

    // 2. Static records (exact match)
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

    // 2b. Static records (wildcard: *.example.com matches foo.example.com)
    if let Some(dot_pos) = name.find('.') {
        let wildcard = format!("*.{}", &name[dot_pos + 1..]);
        for static_rec in &config.static_records {
            if static_rec.name.to_lowercase() == wildcard {
                let matching_type = match static_rec.record_type.to_uppercase().as_str() {
                    "A" => RecordType::A,
                    "AAAA" => RecordType::AAAA,
                    "CNAME" => RecordType::CNAME,
                    _ => continue,
                };

                if qtype == matching_type || qtype == RecordType::ANY {
                    if let Some(record) = parse_static_record(name, static_rec, matching_type) {
                        debug!("Resolved {} via wildcard static record ({})", name, wildcard);
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
    }

    // 3. Wildcard local domain (*.mynetwk.biz -> server IP, fallback for unknown hosts)
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

            // Local domain is authoritative for ALL record types — return NODATA
            // instead of forwarding upstream. This prevents HTTPS/SVCB (type 65)
            // queries from leaking to Cloudflare, which would advertise h3 ALPN
            // and cause ERR_QUIC_PROTOCOL_ERROR on LAN (our proxy doesn't speak QUIC).
            return ResolveResult {
                records: vec![],
                rcode: RCODE_NOERROR,
                cached: false,
                blocked: false,
            };
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

    // 5. Cache lookup (including negative cache)
    if let Some((cached_records, is_negative)) = state_read.dns_cache.get_with_negative(name, qtype).await {
        if is_negative {
            debug!("Resolved {} via negative cache (NXDOMAIN)", name);
            return ResolveResult {
                records: vec![],
                rcode: RCODE_NXDOMAIN,
                cached: true,
                blocked: false,
            };
        }
        debug!("Resolved {} via cache ({} records)", name, cached_records.len());
        return ResolveResult {
            records: cached_records,
            rcode: RCODE_NOERROR,
            cached: true,
            blocked: false,
        };
    }

    // 6. Upstream forward
    let forward_bytes = build_forward_query(query);

    match state_read.upstream.forward(&forward_bytes).await {
        Ok(response_bytes) => {
            match packet::parse_response_sections(&response_bytes) {
                Ok(parsed) => {
                    let rcode = parsed.header.rcode();

                    // Cache only answer records (not authority/additional)
                    // OPT records already filtered by parse_response_sections
                    if !parsed.answers.is_empty() {
                        state_read.dns_cache.insert(name, qtype, &parsed.answers).await;
                    } else if rcode == RCODE_NXDOMAIN || (rcode == RCODE_NOERROR && parsed.answers.is_empty()) {
                        // Negative caching (RFC 2308): cache NXDOMAIN/NODATA
                        // Extract TTL from SOA record in authority section
                        let neg_ttl = extract_soa_negative_ttl(&parsed.authority);
                        if neg_ttl > 0 {
                            state_read.dns_cache.insert_negative(name, qtype, neg_ttl).await;
                        }
                    }

                    debug!("Resolved {} via upstream ({} answers, rcode={})", name, parsed.answers.len(), rcode);
                    ResolveResult {
                        records: parsed.answers,
                        rcode,
                        cached: false,
                        blocked: false,
                    }
                }
                Err(e) => {
                    warn!("Failed to parse upstream response for {}: {}", name, e);
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
    buf.extend_from_slice(&1u16.to_be_bytes()); // AR = 1 (OPT record)

    // Question section
    buf.extend_from_slice(&query.raw_question_bytes);

    // EDNS0 OPT record (RFC 6891)
    // NAME: root (0x00)
    buf.push(0x00);
    // TYPE: OPT (41)
    buf.extend_from_slice(&41u16.to_be_bytes());
    // CLASS: UDP payload size (1232 bytes — safe for most paths, avoids fragmentation)
    buf.extend_from_slice(&1232u16.to_be_bytes());
    // TTL: extended RCODE (0) + version (0) + flags (0, no DO bit)
    buf.extend_from_slice(&0u32.to_be_bytes());
    // RDLENGTH: 0 (no options)
    buf.extend_from_slice(&0u16.to_be_bytes());

    buf
}

/// Extract the negative caching TTL from a SOA record in the authority section.
/// Per RFC 2308: negative TTL = min(SOA.MINIMUM, SOA record TTL).
fn extract_soa_negative_ttl(authority: &[DnsRecord]) -> u32 {
    for record in authority {
        if let RData::SOA { minimum, .. } = &record.rdata {
            return (*minimum).min(record.ttl);
        }
    }
    0 // No SOA found — don't cache negative response (RFC 2308)
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
