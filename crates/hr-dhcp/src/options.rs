use std::net::Ipv4Addr;

/// DHCP option codes (RFC 2132)
pub const OPT_SUBNET_MASK: u8 = 1;
pub const OPT_ROUTER: u8 = 3;
pub const OPT_DNS_SERVER: u8 = 6;
pub const OPT_HOSTNAME: u8 = 12;
pub const OPT_DOMAIN_NAME: u8 = 15;
pub const OPT_BROADCAST_ADDR: u8 = 28;
pub const OPT_REQUESTED_IP: u8 = 50;
pub const OPT_LEASE_TIME: u8 = 51;
pub const OPT_MSG_TYPE: u8 = 53;
pub const OPT_SERVER_ID: u8 = 54;
pub const OPT_PARAM_REQUEST: u8 = 55;
pub const OPT_CLIENT_ID: u8 = 61;
pub const OPT_END: u8 = 255;
pub const OPT_PAD: u8 = 0;

/// DHCP message types
pub const DHCPDISCOVER: u8 = 1;
pub const DHCPOFFER: u8 = 2;
pub const DHCPREQUEST: u8 = 3;
pub const DHCPDECLINE: u8 = 4;
pub const DHCPACK: u8 = 5;
pub const DHCPNAK: u8 = 6;
pub const DHCPRELEASE: u8 = 7;
pub const DHCPINFORM: u8 = 8;

/// A parsed DHCP option
#[derive(Debug, Clone)]
pub struct DhcpOption {
    pub code: u8,
    pub data: Vec<u8>,
}

impl DhcpOption {
    pub fn new(code: u8, data: Vec<u8>) -> Self {
        Self { code, data }
    }

    pub fn msg_type(t: u8) -> Self {
        Self::new(OPT_MSG_TYPE, vec![t])
    }

    pub fn server_id(ip: Ipv4Addr) -> Self {
        Self::new(OPT_SERVER_ID, ip.octets().to_vec())
    }

    pub fn lease_time(secs: u32) -> Self {
        Self::new(OPT_LEASE_TIME, secs.to_be_bytes().to_vec())
    }

    pub fn subnet_mask(mask: Ipv4Addr) -> Self {
        Self::new(OPT_SUBNET_MASK, mask.octets().to_vec())
    }

    pub fn router(ip: Ipv4Addr) -> Self {
        Self::new(OPT_ROUTER, ip.octets().to_vec())
    }

    pub fn dns_server(ip: Ipv4Addr) -> Self {
        Self::new(OPT_DNS_SERVER, ip.octets().to_vec())
    }

    pub fn domain_name(name: &str) -> Self {
        Self::new(OPT_DOMAIN_NAME, name.as_bytes().to_vec())
    }

    pub fn hostname(name: &str) -> Self {
        Self::new(OPT_HOSTNAME, name.as_bytes().to_vec())
    }

    pub fn broadcast(ip: Ipv4Addr) -> Self {
        Self::new(OPT_BROADCAST_ADDR, ip.octets().to_vec())
    }

    /// Extract IPv4 address from option data
    pub fn as_ipv4(&self) -> Option<Ipv4Addr> {
        if self.data.len() == 4 {
            Some(Ipv4Addr::new(self.data[0], self.data[1], self.data[2], self.data[3]))
        } else {
            None
        }
    }

    /// Extract u32 from option data
    pub fn as_u32(&self) -> Option<u32> {
        if self.data.len() == 4 {
            Some(u32::from_be_bytes([self.data[0], self.data[1], self.data[2], self.data[3]]))
        } else {
            None
        }
    }

    /// Extract string from option data
    pub fn as_str(&self) -> Option<String> {
        String::from_utf8(self.data.clone()).ok()
    }

    /// Extract u8 from option data
    pub fn as_u8(&self) -> Option<u8> {
        self.data.first().copied()
    }
}

/// Parse DHCP options from bytes (after magic cookie).
pub fn parse_options(data: &[u8]) -> Vec<DhcpOption> {
    let mut options = Vec::new();
    let mut i = 0;

    while i < data.len() {
        let code = data[i];
        if code == OPT_END {
            break;
        }
        if code == OPT_PAD {
            i += 1;
            continue;
        }

        i += 1;
        if i >= data.len() {
            break;
        }

        let len = data[i] as usize;
        i += 1;

        if i + len > data.len() {
            break;
        }

        options.push(DhcpOption::new(code, data[i..i + len].to_vec()));
        i += len;
    }

    options
}

/// Encode DHCP options to bytes.
pub fn encode_options(options: &[DhcpOption]) -> Vec<u8> {
    let mut buf = Vec::new();
    for opt in options {
        buf.push(opt.code);
        buf.push(opt.data.len() as u8);
        buf.extend_from_slice(&opt.data);
    }
    buf.push(OPT_END);
    buf
}
