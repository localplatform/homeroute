//! Router Advertisement sender via raw ICMPv6 socket.

use std::net::{Ipv6Addr, SocketAddrV6};
use anyhow::Result;
use socket2::{Domain, Protocol, Socket, Type};
use tracing::{info, warn};

use crate::config::Ipv6Config;

/// Build an ICMPv6 Router Advertisement packet.
fn build_ra_packet(config: &Ipv6Config) -> Vec<u8> {
    let mut buf = Vec::with_capacity(128);

    // ICMPv6 header
    buf.push(134); // Type: Router Advertisement
    buf.push(0);   // Code
    buf.extend_from_slice(&[0, 0]); // Checksum (kernel computes for us)

    // RA fields
    buf.push(64);  // Cur Hop Limit
    // Flags: M=managed, O=other
    let flags = if config.ra_managed_flag { 0x80 } else { 0 }
        | if config.ra_other_flag { 0x40 } else { 0 };
    buf.push(flags);
    buf.extend_from_slice(&config.ra_lifetime_secs.to_be_bytes()[2..4]); // Router Lifetime (16-bit)
    buf.extend_from_slice(&0u32.to_be_bytes()); // Reachable Time
    buf.extend_from_slice(&0u32.to_be_bytes()); // Retrans Timer

    // Prefix Information Option (type=3, length=4 = 32 bytes)
    if !config.ra_prefix.is_empty() {
        if let Some((prefix, prefix_len)) = parse_prefix(&config.ra_prefix) {
            buf.push(3);   // Type: Prefix Information
            buf.push(4);   // Length: 4 (in units of 8 bytes = 32 bytes)
            buf.push(prefix_len);
            buf.push(0xC0); // Flags: L=1 (on-link), A=1 (autonomous)
            buf.extend_from_slice(&86400u32.to_be_bytes()); // Valid Lifetime
            buf.extend_from_slice(&14400u32.to_be_bytes()); // Preferred Lifetime
            buf.extend_from_slice(&0u32.to_be_bytes()); // Reserved
            buf.extend_from_slice(&prefix.octets()); // Prefix (16 bytes)
        }
    }

    // RDNSS Option (type=25) — Recursive DNS Server
    for dns_str in &config.dhcpv6_dns_servers {
        if let Ok(dns_ip) = dns_str.parse::<Ipv6Addr>() {
            buf.push(25);  // Type: RDNSS
            buf.push(3);   // Length: 3 (= 24 bytes: 8 header + 16 address)
            buf.extend_from_slice(&[0, 0]); // Reserved
            buf.extend_from_slice(&config.ra_lifetime_secs.to_be_bytes()); // Lifetime
            buf.extend_from_slice(&dns_ip.octets());
        }
    }

    buf
}

fn parse_prefix(prefix_str: &str) -> Option<(Ipv6Addr, u8)> {
    let parts: Vec<&str> = prefix_str.split('/').collect();
    if parts.len() != 2 {
        return None;
    }
    let addr: Ipv6Addr = parts[0].parse().ok()?;
    let len: u8 = parts[1].parse().ok()?;
    Some((addr, len))
}

/// Send periodic Router Advertisements.
pub async fn run_ra_sender(config: Ipv6Config) -> Result<()> {
    if !config.ra_enabled {
        info!("Router Advertisements disabled");
        return Ok(());
    }

    info!("Starting Router Advertisement sender for prefix {}", config.ra_prefix);

    let socket = Socket::new(Domain::IPV6, Type::RAW, Some(Protocol::ICMPV6))?;

    // Set hop limit to 255 (required for RA)
    socket.set_multicast_hops_v6(255)?;

    // Bind to interface
    if !config.interface.is_empty() {
        #[cfg(target_os = "linux")]
        socket.bind_device(Some(config.interface.as_bytes()))?;
    }

    socket.set_nonblocking(true)?;
    let socket = tokio::net::UdpSocket::from_std(socket.into())?;

    let ra_packet = build_ra_packet(&config);

    // Destination: ff02::1 (all-nodes multicast)
    let dest = SocketAddrV6::new(
        "ff02::1".parse().unwrap(),
        0,
        0,
        0,
    );

    // Send interval: ra_lifetime / 3 (per RFC recommendation), min 200s
    let interval_secs = (config.ra_lifetime_secs / 3).max(200);

    info!("RA sender: sending every {}s to ff02::1", interval_secs);

    loop {
        match socket.send_to(&ra_packet, std::net::SocketAddr::V6(dest)).await {
            Ok(_) => {
                info!("Sent Router Advertisement ({} bytes)", ra_packet.len());
            }
            Err(e) => {
                warn!("Failed to send RA: {}", e);
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(interval_secs as u64)).await;
    }
}
