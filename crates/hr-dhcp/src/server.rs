use std::net::{Ipv4Addr, SocketAddr};
use anyhow::Result;
use socket2::{Domain, Protocol, Socket, Type};
use tracing::{debug, info, warn};

use crate::SharedDhcpState;
use crate::options::DHCPNAK;
use crate::packet::DhcpPacket;
use crate::state_machine;

/// Run the DHCP server on port 67.
/// Uses raw UDP socket with SO_BROADCAST for DHCP broadcast responses.
pub async fn run_dhcp_server(state: SharedDhcpState) -> Result<()> {
    let config = state.read().await.config.clone();

    if !config.enabled {
        info!("DHCP server disabled");
        return Ok(());
    }

    // Create socket with SO_BROADCAST
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.set_broadcast(true)?;

    // Bind to 0.0.0.0:67
    let addr: SocketAddr = "0.0.0.0:67".parse().unwrap();
    socket.bind(&addr.into())?;

    // Bind to specific interface if configured
    #[cfg(target_os = "linux")]
    if !config.interface.is_empty() {
        socket.bind_device(Some(config.interface.as_bytes()))?;
        info!("DHCP bound to interface {}", config.interface);
    }

    socket.set_nonblocking(true)?;
    let socket = tokio::net::UdpSocket::from_std(socket.into())?;

    info!("DHCP server listening on 0.0.0.0:67");

    let mut buf = [0u8; 1500];

    loop {
        let (len, src) = match socket.recv_from(&mut buf).await {
            Ok(r) => r,
            Err(e) => {
                warn!("DHCP recv error: {}", e);
                continue;
            }
        };

        let packet_data = &buf[..len];

        let packet = match DhcpPacket::parse(packet_data) {
            Ok(p) => p,
            Err(e) => {
                debug!("Invalid DHCP packet from {}: {}", src, e);
                continue;
            }
        };

        // Only handle BOOTREQUEST (op=1)
        if packet.op != 1 {
            continue;
        }

        let mut state_write = state.write().await;
        let config = state_write.config.clone();
        let server_ip = state_write.server_ip;

        let response = state_machine::handle_dhcp_packet(
            &packet,
            &config,
            &mut state_write.lease_store,
            server_ip,
        );

        drop(state_write);

        if let Some(response) = response {
            let response_bytes = response.to_bytes();

            // Determine destination: broadcast or unicast
            // RFC 2131 §4.3.2: DHCPNAK MUST always be broadcast when giaddr is zero.
            let dest = if response.msg_type() == Some(DHCPNAK) {
                SocketAddr::new("255.255.255.255".parse().unwrap(), 68)
            } else if packet.is_broadcast() || packet.ciaddr == Ipv4Addr::UNSPECIFIED {
                SocketAddr::new("255.255.255.255".parse().unwrap(), 68)
            } else {
                SocketAddr::new(packet.ciaddr.into(), 68)
            };

            if let Err(e) = socket.send_to(&response_bytes, dest).await {
                warn!("Failed to send DHCP response to {}: {}", dest, e);
            }
        }
    }
}
