//! Stateless DHCPv6 server (RFC 8415, stateless mode).
//! Only handles INFORMATION-REQUEST messages — provides DNS server option.

use std::net::Ipv6Addr;
use anyhow::Result;
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

use crate::config::Ipv6Config;

// DHCPv6 message types
const MSG_INFORMATION_REQUEST: u8 = 11;
const MSG_REPLY: u8 = 7;

// DHCPv6 option codes
const OPT_CLIENTID: u16 = 1;
const OPT_SERVERID: u16 = 2;
const OPT_DNS_SERVERS: u16 = 23;
const OPT_DOMAIN_LIST: u16 = 24;

/// Run the stateless DHCPv6 server on port 547.
pub async fn run_dhcpv6_server(config: Ipv6Config) -> Result<()> {
    if !config.dhcpv6_enabled {
        info!("DHCPv6 server disabled");
        return Ok(());
    }

    let bind_addr = format!("[::]:{}", 547);
    let socket = UdpSocket::bind(&bind_addr).await?;
    info!("DHCPv6 server listening on {}", bind_addr);

    let mut buf = [0u8; 1500];

    loop {
        let (len, src) = match socket.recv_from(&mut buf).await {
            Ok(r) => r,
            Err(e) => {
                warn!("DHCPv6 recv error: {}", e);
                continue;
            }
        };

        if len < 4 {
            continue;
        }

        let msg_type = buf[0];

        // Only handle INFORMATION-REQUEST (stateless mode)
        if msg_type != MSG_INFORMATION_REQUEST {
            debug!("Ignoring DHCPv6 message type {}", msg_type);
            continue;
        }

        let transaction_id = [buf[1], buf[2], buf[3]];

        // Parse client ID from request (if present)
        let client_id = extract_option(&buf[4..len], OPT_CLIENTID);

        // Build REPLY
        let reply = build_reply(&transaction_id, client_id.as_deref(), &config);

        if let Err(e) = socket.send_to(&reply, src).await {
            warn!("Failed to send DHCPv6 reply to {}: {}", src, e);
        } else {
            debug!("Sent DHCPv6 reply to {} ({} bytes)", src, reply.len());
        }
    }
}

fn extract_option(data: &[u8], option_code: u16) -> Option<Vec<u8>> {
    let mut offset = 0;
    while offset + 4 <= data.len() {
        let code = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let len = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
        offset += 4;

        if offset + len > data.len() {
            break;
        }

        if code == option_code {
            return Some(data[offset..offset + len].to_vec());
        }

        offset += len;
    }
    None
}

fn build_reply(transaction_id: &[u8; 3], client_id: Option<&[u8]>, config: &Ipv6Config) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);

    // Message type + Transaction ID
    buf.push(MSG_REPLY);
    buf.extend_from_slice(transaction_id);

    // Echo Client ID if present
    if let Some(cid) = client_id {
        buf.extend_from_slice(&OPT_CLIENTID.to_be_bytes());
        buf.extend_from_slice(&(cid.len() as u16).to_be_bytes());
        buf.extend_from_slice(cid);
    }

    // Server ID (DUID-LLT with a simple identifier)
    let server_duid: &[u8] = &[0, 3, 0, 1, 0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
    buf.extend_from_slice(&OPT_SERVERID.to_be_bytes());
    buf.extend_from_slice(&(server_duid.len() as u16).to_be_bytes());
    buf.extend_from_slice(server_duid);

    // DNS Recursive Name Server option (23)
    let dns_addrs: Vec<Ipv6Addr> = config
        .dhcpv6_dns_servers
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();

    if !dns_addrs.is_empty() {
        let data_len = dns_addrs.len() * 16;
        buf.extend_from_slice(&OPT_DNS_SERVERS.to_be_bytes());
        buf.extend_from_slice(&(data_len as u16).to_be_bytes());
        for addr in &dns_addrs {
            buf.extend_from_slice(&addr.octets());
        }
    }

    // Domain Search List option (24) — encode using DNS label format
    // For simplicity, encode the domain from IPv6 config
    // (We use the same domain as the DHCP config)
    // This is optional and can be enhanced later

    buf
}
