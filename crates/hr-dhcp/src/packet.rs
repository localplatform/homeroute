//! DHCPv4 packet parser/serializer (RFC 2131)

use std::net::Ipv4Addr;
use thiserror::Error;

use crate::options::{self, DhcpOption, OPT_MSG_TYPE, OPT_REQUESTED_IP, OPT_SERVER_ID, OPT_HOSTNAME, OPT_CLIENT_ID};

/// DHCP magic cookie
pub const MAGIC_COOKIE: [u8; 4] = [0x63, 0x82, 0x53, 0x63];

#[derive(Error, Debug)]
pub enum DhcpParseError {
    #[error("Packet too short: {0} bytes (minimum 240)")]
    TooShort(usize),
    #[error("Invalid magic cookie")]
    InvalidMagic,
}

/// Parsed DHCPv4 packet
#[derive(Debug, Clone)]
pub struct DhcpPacket {
    pub op: u8,
    pub htype: u8,
    pub hlen: u8,
    pub hops: u8,
    pub xid: u32,
    pub secs: u16,
    pub flags: u16,
    pub ciaddr: Ipv4Addr,
    pub yiaddr: Ipv4Addr,
    pub siaddr: Ipv4Addr,
    pub giaddr: Ipv4Addr,
    pub chaddr: [u8; 16],
    pub sname: [u8; 64],
    pub file: [u8; 128],
    pub options: Vec<DhcpOption>,
}

impl DhcpPacket {
    /// Parse a DHCP packet from raw bytes
    pub fn parse(data: &[u8]) -> Result<Self, DhcpParseError> {
        if data.len() < 240 {
            return Err(DhcpParseError::TooShort(data.len()));
        }

        // Verify magic cookie at offset 236
        if data[236..240] != MAGIC_COOKIE {
            return Err(DhcpParseError::InvalidMagic);
        }

        let mut chaddr = [0u8; 16];
        chaddr.copy_from_slice(&data[28..44]);
        let mut sname = [0u8; 64];
        sname.copy_from_slice(&data[44..108]);
        let mut file = [0u8; 128];
        file.copy_from_slice(&data[108..236]);

        let options = if data.len() > 240 {
            options::parse_options(&data[240..])
        } else {
            vec![]
        };

        Ok(DhcpPacket {
            op: data[0],
            htype: data[1],
            hlen: data[2],
            hops: data[3],
            xid: u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
            secs: u16::from_be_bytes([data[8], data[9]]),
            flags: u16::from_be_bytes([data[10], data[11]]),
            ciaddr: Ipv4Addr::new(data[12], data[13], data[14], data[15]),
            yiaddr: Ipv4Addr::new(data[16], data[17], data[18], data[19]),
            siaddr: Ipv4Addr::new(data[20], data[21], data[22], data[23]),
            giaddr: Ipv4Addr::new(data[24], data[25], data[26], data[27]),
            chaddr,
            sname,
            file,
            options,
        })
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(576);

        buf.push(self.op);
        buf.push(self.htype);
        buf.push(self.hlen);
        buf.push(self.hops);
        buf.extend_from_slice(&self.xid.to_be_bytes());
        buf.extend_from_slice(&self.secs.to_be_bytes());
        buf.extend_from_slice(&self.flags.to_be_bytes());
        buf.extend_from_slice(&self.ciaddr.octets());
        buf.extend_from_slice(&self.yiaddr.octets());
        buf.extend_from_slice(&self.siaddr.octets());
        buf.extend_from_slice(&self.giaddr.octets());
        buf.extend_from_slice(&self.chaddr);
        buf.extend_from_slice(&self.sname);
        buf.extend_from_slice(&self.file);
        buf.extend_from_slice(&MAGIC_COOKIE);

        let opt_bytes = options::encode_options(&self.options);
        buf.extend_from_slice(&opt_bytes);

        // Pad to minimum 300 bytes (common DHCP minimum)
        while buf.len() < 300 {
            buf.push(0);
        }

        buf
    }

    /// Get MAC address as a formatted string (aa:bb:cc:dd:ee:ff)
    pub fn mac_str(&self) -> String {
        let len = self.hlen.min(16) as usize;
        self.chaddr[..len]
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(":")
    }

    /// Get MAC address as bytes
    pub fn mac_bytes(&self) -> &[u8] {
        let len = self.hlen.min(16) as usize;
        &self.chaddr[..len]
    }

    /// Find option by code
    pub fn get_option(&self, code: u8) -> Option<&DhcpOption> {
        self.options.iter().find(|o| o.code == code)
    }

    /// Get DHCP message type
    pub fn msg_type(&self) -> Option<u8> {
        self.get_option(OPT_MSG_TYPE)?.as_u8()
    }

    /// Get requested IP address
    pub fn requested_ip(&self) -> Option<Ipv4Addr> {
        self.get_option(OPT_REQUESTED_IP)?.as_ipv4()
    }

    /// Get server identifier
    pub fn server_id(&self) -> Option<Ipv4Addr> {
        self.get_option(OPT_SERVER_ID)?.as_ipv4()
    }

    /// Get hostname
    pub fn hostname(&self) -> Option<String> {
        self.get_option(OPT_HOSTNAME)?.as_str()
    }

    /// Get client identifier
    pub fn client_id(&self) -> Option<String> {
        let opt = self.get_option(OPT_CLIENT_ID)?;
        Some(
            opt.data
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<Vec<_>>()
                .join(":"),
        )
    }

    /// Check if broadcast flag is set
    pub fn is_broadcast(&self) -> bool {
        self.flags & 0x8000 != 0
    }

    /// Build a reply packet from this request.
    /// `ciaddr` should be set from the client's ciaddr for DHCPACK (RFC 2131 §4.3.1).
    pub fn build_reply(
        &self,
        msg_type: u8,
        yiaddr: Ipv4Addr,
        siaddr: Ipv4Addr,
        ciaddr: Ipv4Addr,
        options: Vec<DhcpOption>,
    ) -> DhcpPacket {
        DhcpPacket {
            op: 2, // BOOTREPLY
            htype: self.htype,
            hlen: self.hlen,
            hops: 0,
            xid: self.xid,
            secs: 0,
            flags: self.flags,
            ciaddr,
            yiaddr,
            siaddr,
            giaddr: self.giaddr,
            chaddr: self.chaddr,
            sname: [0u8; 64],
            file: [0u8; 128],
            options: {
                let mut opts = vec![DhcpOption::msg_type(msg_type)];
                opts.extend(options);
                opts
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_discover() -> Vec<u8> {
        let mut pkt = vec![0u8; 300];
        pkt[0] = 1; // BOOTREQUEST
        pkt[1] = 1; // Ethernet
        pkt[2] = 6; // MAC length
        // XID
        pkt[4..8].copy_from_slice(&0x12345678u32.to_be_bytes());
        // MAC address
        pkt[28] = 0xAA;
        pkt[29] = 0xBB;
        pkt[30] = 0xCC;
        pkt[31] = 0xDD;
        pkt[32] = 0xEE;
        pkt[33] = 0xFF;
        // Magic cookie
        pkt[236..240].copy_from_slice(&MAGIC_COOKIE);
        // Options: DHCP Message Type = DISCOVER
        pkt[240] = 53;
        pkt[241] = 1;
        pkt[242] = 1; // DISCOVER
        pkt[243] = 255; // END
        pkt
    }

    #[test]
    fn test_parse_discover() {
        let data = make_discover();
        let pkt = DhcpPacket::parse(&data).unwrap();
        assert_eq!(pkt.op, 1);
        assert_eq!(pkt.xid, 0x12345678);
        assert_eq!(pkt.mac_str(), "aa:bb:cc:dd:ee:ff");
        assert_eq!(pkt.msg_type(), Some(1));
    }

    #[test]
    fn test_roundtrip() {
        let data = make_discover();
        let pkt = DhcpPacket::parse(&data).unwrap();
        let serialized = pkt.to_bytes();
        let pkt2 = DhcpPacket::parse(&serialized).unwrap();
        assert_eq!(pkt2.xid, pkt.xid);
        assert_eq!(pkt2.mac_str(), pkt.mac_str());
        assert_eq!(pkt2.msg_type(), pkt.msg_type());
    }

    #[test]
    fn test_build_reply() {
        let data = make_discover();
        let request = DhcpPacket::parse(&data).unwrap();
        let reply = request.build_reply(
            2, // OFFER
            Ipv4Addr::new(10, 0, 0, 100),
            Ipv4Addr::new(10, 0, 0, 254),
            Ipv4Addr::UNSPECIFIED,
            vec![
                DhcpOption::lease_time(86400),
                DhcpOption::subnet_mask(Ipv4Addr::new(255, 255, 255, 0)),
            ],
        );
        assert_eq!(reply.op, 2);
        assert_eq!(reply.yiaddr, Ipv4Addr::new(10, 0, 0, 100));
        assert_eq!(reply.msg_type(), Some(2));
    }
}
