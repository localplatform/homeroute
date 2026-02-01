use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

/// DNS record types we support
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordType {
    A,
    NS,
    CNAME,
    SOA,
    PTR,
    MX,
    TXT,
    AAAA,
    SRV,
    ANY,
    Unknown(u16),
}

impl RecordType {
    pub fn from_u16(val: u16) -> Self {
        match val {
            1 => Self::A,
            2 => Self::NS,
            5 => Self::CNAME,
            6 => Self::SOA,
            12 => Self::PTR,
            15 => Self::MX,
            16 => Self::TXT,
            28 => Self::AAAA,
            33 => Self::SRV,
            255 => Self::ANY,
            other => Self::Unknown(other),
        }
    }

    pub fn to_u16(self) -> u16 {
        match self {
            Self::A => 1,
            Self::NS => 2,
            Self::CNAME => 5,
            Self::SOA => 6,
            Self::PTR => 12,
            Self::MX => 15,
            Self::TXT => 16,
            Self::AAAA => 28,
            Self::SRV => 33,
            Self::ANY => 255,
            Self::Unknown(v) => v,
        }
    }
}

// Need manual impl because of Unknown variant
#[allow(unreachable_patterns)]
impl fmt::Display for RecordType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::A => write!(f, "A"),
            Self::NS => write!(f, "NS"),
            Self::CNAME => write!(f, "CNAME"),
            Self::SOA => write!(f, "SOA"),
            Self::PTR => write!(f, "PTR"),
            Self::MX => write!(f, "MX"),
            Self::TXT => write!(f, "TXT"),
            Self::AAAA => write!(f, "AAAA"),
            Self::SRV => write!(f, "SRV"),
            Self::ANY => write!(f, "ANY"),
            Self::Unknown(v) => write!(f, "TYPE{}", v),
        }
    }
}

/// DNS record class
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordClass {
    IN,
    Any,
    Unknown(u16),
}

impl RecordClass {
    pub fn from_u16(val: u16) -> Self {
        match val {
            1 => Self::IN,
            255 => Self::Any,
            other => Self::Unknown(other),
        }
    }

    pub fn to_u16(self) -> u16 {
        match self {
            Self::IN => 1,
            Self::Any => 255,
            Self::Unknown(v) => v,
        }
    }
}

/// DNS resource record data
#[derive(Debug, Clone)]
pub enum RData {
    A(Ipv4Addr),
    AAAA(Ipv6Addr),
    CNAME(String),
    PTR(String),
    NS(String),
    MX { preference: u16, exchange: String },
    TXT(String),
    SOA {
        mname: String,
        rname: String,
        serial: u32,
        refresh: u32,
        retry: u32,
        expire: u32,
        minimum: u32,
    },
    SRV {
        priority: u16,
        weight: u16,
        port: u16,
        target: String,
    },
    /// Raw bytes for unknown record types
    Raw(Vec<u8>),
}

/// A complete DNS resource record
#[derive(Debug, Clone)]
pub struct DnsRecord {
    pub name: String,
    pub rtype: RecordType,
    pub class: RecordClass,
    pub ttl: u32,
    pub rdata: RData,
}

impl DnsRecord {
    pub fn a(name: &str, ip: Ipv4Addr, ttl: u32) -> Self {
        Self {
            name: name.to_string(),
            rtype: RecordType::A,
            class: RecordClass::IN,
            ttl,
            rdata: RData::A(ip),
        }
    }

    pub fn aaaa(name: &str, ip: Ipv6Addr, ttl: u32) -> Self {
        Self {
            name: name.to_string(),
            rtype: RecordType::AAAA,
            class: RecordClass::IN,
            ttl,
            rdata: RData::AAAA(ip),
        }
    }

    pub fn cname(name: &str, target: &str, ttl: u32) -> Self {
        Self {
            name: name.to_string(),
            rtype: RecordType::CNAME,
            class: RecordClass::IN,
            ttl,
            rdata: RData::CNAME(target.to_string()),
        }
    }

    pub fn ptr(name: &str, target: &str, ttl: u32) -> Self {
        Self {
            name: name.to_string(),
            rtype: RecordType::PTR,
            class: RecordClass::IN,
            ttl,
            rdata: RData::PTR(target.to_string()),
        }
    }
}
