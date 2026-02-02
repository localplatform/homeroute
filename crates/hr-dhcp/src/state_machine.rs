use std::net::Ipv4Addr;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

use crate::config::DhcpConfig;
use crate::lease_store::{Lease, LeaseStore};
use crate::options::*;
use crate::packet::DhcpPacket;

/// Handle an incoming DHCP packet and produce a response (if any).
pub fn handle_dhcp_packet(
    packet: &DhcpPacket,
    config: &DhcpConfig,
    lease_store: &mut LeaseStore,
    server_ip: Ipv4Addr,
) -> Option<DhcpPacket> {
    let msg_type = packet.msg_type()?;

    match msg_type {
        DHCPDISCOVER => handle_discover(packet, config, lease_store, server_ip),
        DHCPREQUEST => handle_request(packet, config, lease_store, server_ip),
        DHCPRELEASE => {
            handle_release(packet, lease_store);
            None
        }
        DHCPINFORM => handle_inform(packet, config, server_ip),
        DHCPDECLINE => {
            handle_decline(packet, lease_store);
            None
        }
        _ => {
            debug!("Ignoring DHCP message type {}", msg_type);
            None
        }
    }
}

fn handle_discover(
    packet: &DhcpPacket,
    config: &DhcpConfig,
    lease_store: &mut LeaseStore,
    server_ip: Ipv4Addr,
) -> Option<DhcpPacket> {
    let mac = packet.mac_str();
    info!("DHCPDISCOVER from {}", mac);

    let range_start: Ipv4Addr = config.range_start.parse().ok()?;
    let range_end: Ipv4Addr = config.range_end.parse().ok()?;

    let static_leases: Vec<(String, Ipv4Addr, String)> = config
        .static_leases
        .iter()
        .filter_map(|s| {
            let ip: Ipv4Addr = s.ip.parse().ok()?;
            Some((s.mac.to_lowercase(), ip, s.hostname.clone()))
        })
        .collect();

    let (offered_ip, hostname) =
        lease_store.allocate_ip(&mac, range_start, range_end, &static_leases)?;

    info!("DHCPOFFER {} to {}", offered_ip, mac);

    // Reserve IP with a short lease (60s) to prevent double-offering.
    // The real lease is committed at REQUEST time with the full duration.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    lease_store.add_lease(Lease {
        expiry: now + 60,
        mac: mac.clone(),
        ip: offered_ip,
        hostname: hostname.clone(),
        client_id: packet.client_id(),
    });

    let mut options = build_standard_options(config, server_ip);

    if let Some(ref h) = hostname {
        options.push(DhcpOption::hostname(h));
    }

    // DHCPOFFER: ciaddr is always 0 (RFC 2131 §4.3.1)
    Some(packet.build_reply(DHCPOFFER, offered_ip, server_ip, Ipv4Addr::UNSPECIFIED, options))
}

fn handle_request(
    packet: &DhcpPacket,
    config: &DhcpConfig,
    lease_store: &mut LeaseStore,
    server_ip: Ipv4Addr,
) -> Option<DhcpPacket> {
    let mac = packet.mac_str();

    // Check if this REQUEST is for us (server identifier matches)
    if let Some(requested_server) = packet.server_id() {
        if requested_server != server_ip {
            debug!("DHCPREQUEST from {} for different server {}", mac, requested_server);
            return None;
        }
    }

    // Determine the requested IP
    let requested_ip = packet
        .requested_ip()
        .or(if packet.ciaddr != Ipv4Addr::UNSPECIFIED {
            Some(packet.ciaddr)
        } else {
            None
        });

    let requested_ip = match requested_ip {
        Some(ip) => ip,
        None => {
            warn!("DHCPREQUEST from {} without requested IP", mac);
            return Some(build_nak(packet, server_ip));
        }
    };

    info!("DHCPREQUEST from {} for {}", mac, requested_ip);

    // RFC 2131 §4.3.2: Detect INIT-REBOOT state (no server_id, requested_ip set, ciaddr=0).
    // If the server has no record of this client, it MUST remain silent.
    let is_init_reboot = packet.server_id().is_none()
        && packet.requested_ip().is_some()
        && packet.ciaddr == Ipv4Addr::UNSPECIFIED;

    if is_init_reboot && lease_store.get_lease_by_mac(&mac).is_none() {
        debug!("INIT-REBOOT from {} for {} — no record, staying silent", mac, requested_ip);
        return None;
    }

    // Validate the request
    let range_start: Ipv4Addr = config.range_start.parse().ok()?;
    let range_end: Ipv4Addr = config.range_end.parse().ok()?;

    let is_static = config
        .static_leases
        .iter()
        .any(|s| s.mac.to_lowercase() == mac && s.ip.parse::<Ipv4Addr>().ok() == Some(requested_ip));

    let ip_u32 = u32::from(requested_ip);
    let in_range = ip_u32 >= u32::from(range_start) && ip_u32 <= u32::from(range_end);

    if !is_static && !in_range {
        warn!("DHCPNAK: {} requested {} which is out of range", mac, requested_ip);
        return Some(build_nak(packet, server_ip));
    }

    // Check if IP is in use by a different MAC
    if let Some(existing) = lease_store.get_lease(requested_ip) {
        if existing.mac != mac {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if existing.expiry > now {
                warn!("DHCPNAK: {} requested {} which is leased to {}", mac, requested_ip, existing.mac);
                return Some(build_nak(packet, server_ip));
            }
        }
    }

    // Commit the lease
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let hostname = packet.hostname().or_else(|| {
        config
            .static_leases
            .iter()
            .find(|s| s.mac.to_lowercase() == mac)
            .map(|s| s.hostname.clone())
            .filter(|h| !h.is_empty())
    });

    lease_store.add_lease(Lease {
        expiry: now + config.default_lease_time_secs,
        mac: mac.clone(),
        ip: requested_ip,
        hostname: hostname.clone(),
        client_id: packet.client_id(),
    });

    info!("DHCPACK {} to {} (hostname: {:?})", requested_ip, mac, hostname);

    let mut options = build_standard_options(config, server_ip);
    if let Some(ref h) = hostname {
        options.push(DhcpOption::hostname(h));
    }

    // DHCPACK: echo client's ciaddr (RFC 2131 §4.3.1 Table 3)
    Some(packet.build_reply(DHCPACK, requested_ip, server_ip, packet.ciaddr, options))
}

fn handle_release(packet: &DhcpPacket, lease_store: &mut LeaseStore) {
    let mac = packet.mac_str();
    let ip = packet.ciaddr;

    if ip != Ipv4Addr::UNSPECIFIED {
        // Validate that the releasing client actually owns this lease
        if let Some(lease) = lease_store.get_lease(ip) {
            if lease.mac != mac {
                warn!("DHCPRELEASE from {} for {} — MAC mismatch (leased to {})", mac, ip, lease.mac);
                return;
            }
        }
        info!("DHCPRELEASE from {} for {}", mac, ip);
        lease_store.remove_lease(ip);
    }
}

fn handle_inform(
    packet: &DhcpPacket,
    config: &DhcpConfig,
    server_ip: Ipv4Addr,
) -> Option<DhcpPacket> {
    let mac = packet.mac_str();
    info!("DHCPINFORM from {}", mac);

    let options = build_standard_options(config, server_ip);
    // INFORM: yiaddr must be 0, client already has an IP; ciaddr from client
    Some(packet.build_reply(DHCPACK, Ipv4Addr::UNSPECIFIED, server_ip, packet.ciaddr, options))
}

fn handle_decline(packet: &DhcpPacket, lease_store: &mut LeaseStore) {
    let mac = packet.mac_str();
    if let Some(ip) = packet.requested_ip() {
        // Validate that the declining client actually owns this lease
        if let Some(lease) = lease_store.get_lease(ip) {
            if lease.mac != mac {
                warn!("DHCPDECLINE from {} for {} — MAC mismatch (leased to {})", mac, ip, lease.mac);
                return;
            }
        }
        info!("DHCPDECLINE from {} for {}", mac, ip);
        // Remove the lease so the IP can be re-offered.
        // The client detected an ARP conflict -- this is common in container
        // environments where the interface may briefly have stale addresses.
        lease_store.remove_lease(ip);
    }
}

fn build_nak(packet: &DhcpPacket, server_ip: Ipv4Addr) -> DhcpPacket {
    // DHCPNAK: ciaddr and yiaddr are always 0 (RFC 2131 §4.3.2)
    packet.build_reply(
        DHCPNAK,
        Ipv4Addr::UNSPECIFIED,
        server_ip,
        Ipv4Addr::UNSPECIFIED,
        vec![DhcpOption::server_id(server_ip)],
    )
}

fn build_standard_options(config: &DhcpConfig, server_ip: Ipv4Addr) -> Vec<DhcpOption> {
    let lease = config.default_lease_time_secs as u32;
    let mut opts = vec![
        DhcpOption::server_id(server_ip),
        DhcpOption::lease_time(lease),
        DhcpOption::renewal_time(lease / 2),       // T1 = 50% of lease
        DhcpOption::rebinding_time(lease * 7 / 8), // T2 = 87.5% of lease
    ];

    if let Ok(mask) = config.netmask.parse::<Ipv4Addr>() {
        opts.push(DhcpOption::subnet_mask(mask));
    }

    if let Ok(gw) = config.gateway.parse::<Ipv4Addr>() {
        opts.push(DhcpOption::router(gw));
    }

    if let Ok(dns) = config.dns_server.parse::<Ipv4Addr>() {
        opts.push(DhcpOption::dns_server(dns));
    }

    if !config.domain.is_empty() {
        opts.push(DhcpOption::domain_name(&config.domain));
    }

    // Broadcast address: network_address | ~netmask
    if let (Ok(gw), Ok(mask)) = (
        config.gateway.parse::<Ipv4Addr>(),
        config.netmask.parse::<Ipv4Addr>(),
    ) {
        let network = u32::from(gw) & u32::from(mask);
        let broadcast = Ipv4Addr::from(network | !u32::from(mask));
        opts.push(DhcpOption::broadcast(broadcast));
    }

    opts
}
