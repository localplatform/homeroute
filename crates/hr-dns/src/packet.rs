//! DNS wire format parser and serializer (RFC 1035).
//! Zero-copy parsing from &[u8] buffers with minimal allocations.

use thiserror::Error;
use std::net::{Ipv4Addr, Ipv6Addr};

use crate::records::{DnsRecord, RData, RecordClass, RecordType};

#[derive(Error, Debug)]
pub enum DnsParseError {
    #[error("Packet truncated at offset {0}")]
    Truncated(usize),
    #[error("Invalid name label at offset {0}")]
    InvalidLabel(usize),
    #[error("Name compression loop detected")]
    CompressionLoop,
    #[error("Invalid UTF-8 in name")]
    InvalidUtf8,
    #[error("Packet too short: {0} bytes")]
    TooShort(usize),
    #[error("Name too long (exceeds 255 bytes)")]
    NameTooLong,
    #[error("Label too long: {0} bytes (max 63)")]
    LabelTooLong(usize),
}

/// Parsed DNS header (12 bytes)
#[derive(Debug, Clone)]
pub struct DnsHeader {
    pub id: u16,
    pub flags: u16,
    pub qd_count: u16,
    pub an_count: u16,
    pub ns_count: u16,
    pub ar_count: u16,
}

impl DnsHeader {
    // Flag accessors
    pub fn is_response(&self) -> bool {
        self.flags & 0x8000 != 0
    }
    pub fn opcode(&self) -> u8 {
        ((self.flags >> 11) & 0xF) as u8
    }
    pub fn is_authoritative(&self) -> bool {
        self.flags & 0x0400 != 0
    }
    pub fn is_truncated(&self) -> bool {
        self.flags & 0x0200 != 0
    }
    pub fn recursion_desired(&self) -> bool {
        self.flags & 0x0100 != 0
    }
    pub fn recursion_available(&self) -> bool {
        self.flags & 0x0080 != 0
    }
    pub fn rcode(&self) -> u8 {
        (self.flags & 0xF) as u8
    }
}

/// A parsed DNS question
#[derive(Debug, Clone)]
pub struct DnsQuestion {
    pub name: String,
    pub qtype: RecordType,
    pub qclass: RecordClass,
}

/// A fully parsed DNS query (what we receive from clients)
#[derive(Debug, Clone)]
pub struct DnsQuery {
    pub header: DnsHeader,
    pub questions: Vec<DnsQuestion>,
    /// Raw bytes of the question section (for building responses that copy it)
    pub raw_question_bytes: Vec<u8>,
}

/// Parse a DNS name from the wire format with pointer compression support.
/// Returns (name, bytes_consumed_from_start_offset).
pub fn parse_name(buf: &[u8], mut offset: usize) -> Result<(String, usize), DnsParseError> {
    let mut name = String::with_capacity(64);
    let mut jumped = false;
    let mut end_offset = 0;
    let mut jumps = 0;
    const MAX_JUMPS: usize = 10;

    loop {
        if offset >= buf.len() {
            return Err(DnsParseError::Truncated(offset));
        }

        let len = buf[offset] as usize;

        // Pointer (compression)
        if len & 0xC0 == 0xC0 {
            if offset + 1 >= buf.len() {
                return Err(DnsParseError::Truncated(offset));
            }
            if !jumped {
                end_offset = offset + 2;
            }
            offset = ((len & 0x3F) << 8) | (buf[offset + 1] as usize);
            jumped = true;
            jumps += 1;
            if jumps > MAX_JUMPS {
                return Err(DnsParseError::CompressionLoop);
            }
            continue;
        }

        // End of name
        if len == 0 {
            if !jumped {
                end_offset = offset + 1;
            }
            break;
        }

        // Normal label — RFC 1035: labels must be ≤63 octets
        if len > 63 {
            return Err(DnsParseError::LabelTooLong(len));
        }

        offset += 1;
        if offset + len > buf.len() {
            return Err(DnsParseError::Truncated(offset));
        }

        if !name.is_empty() {
            name.push('.');
        }

        let label = std::str::from_utf8(&buf[offset..offset + len])
            .map_err(|_| DnsParseError::InvalidUtf8)?;
        name.push_str(label);
        offset += len;

        // RFC 1035 §2.3.4: total name must not exceed 253 characters (255 wire bytes)
        if name.len() > 253 {
            return Err(DnsParseError::NameTooLong);
        }
    }

    Ok((name, end_offset))
}

/// Encode a DNS name into wire format labels.
/// Labels are clamped to 63 bytes per RFC 1035 §2.3.4.
pub fn encode_name(name: &str, buf: &mut Vec<u8>) {
    if name.is_empty() {
        buf.push(0);
        return;
    }
    for label in name.split('.') {
        let len = label.len().min(63);
        buf.push(len as u8);
        buf.extend_from_slice(&label.as_bytes()[..len]);
    }
    buf.push(0);
}

/// Parse a DNS header from bytes.
fn parse_header(buf: &[u8]) -> Result<DnsHeader, DnsParseError> {
    if buf.len() < 12 {
        return Err(DnsParseError::TooShort(buf.len()));
    }
    Ok(DnsHeader {
        id: u16::from_be_bytes([buf[0], buf[1]]),
        flags: u16::from_be_bytes([buf[2], buf[3]]),
        qd_count: u16::from_be_bytes([buf[4], buf[5]]),
        an_count: u16::from_be_bytes([buf[6], buf[7]]),
        ns_count: u16::from_be_bytes([buf[8], buf[9]]),
        ar_count: u16::from_be_bytes([buf[10], buf[11]]),
    })
}

/// Parse a DNS query packet from raw bytes.
pub fn parse_query(buf: &[u8]) -> Result<DnsQuery, DnsParseError> {
    let header = parse_header(buf)?;
    let mut offset = 12;
    let question_start = offset;
    let mut questions = Vec::with_capacity(header.qd_count as usize);

    for _ in 0..header.qd_count {
        let (name, new_offset) = parse_name(buf, offset)?;
        offset = new_offset;

        if offset + 4 > buf.len() {
            return Err(DnsParseError::Truncated(offset));
        }

        let qtype = RecordType::from_u16(u16::from_be_bytes([buf[offset], buf[offset + 1]]));
        let qclass = RecordClass::from_u16(u16::from_be_bytes([buf[offset + 2], buf[offset + 3]]));
        offset += 4;

        questions.push(DnsQuestion {
            name: name.to_lowercase(),
            qtype,
            qclass,
        });
    }

    Ok(DnsQuery {
        header,
        questions,
        raw_question_bytes: buf[question_start..offset].to_vec(),
    })
}

/// Parsed DNS response with separated sections.
pub struct ParsedResponse {
    pub header: DnsHeader,
    pub answers: Vec<DnsRecord>,
    pub authority: Vec<DnsRecord>,
    pub additional: Vec<DnsRecord>,
}

/// Parse resource records from an upstream response, separating sections.
/// OPT records (type 41) are filtered out per RFC 6891 (must not be cached).
pub fn parse_response_records(buf: &[u8]) -> Result<(DnsHeader, Vec<DnsRecord>), DnsParseError> {
    let parsed = parse_response_sections(buf)?;
    // For backward compatibility, return only answer records
    Ok((parsed.header, parsed.answers))
}

/// Parse a full DNS response into separated sections (answer, authority, additional).
/// Filters out OPT records (type 41) per RFC 6891.
pub fn parse_response_sections(buf: &[u8]) -> Result<ParsedResponse, DnsParseError> {
    let header = parse_header(buf)?;
    let mut offset = 12;

    // Skip questions
    for _ in 0..header.qd_count {
        let (_, new_offset) = parse_name(buf, offset)?;
        offset = new_offset + 4; // skip QTYPE + QCLASS
    }

    let mut answers = Vec::new();
    let mut authority = Vec::new();
    let mut additional = Vec::new();

    let sections: [(usize, u8); 3] = [
        (header.an_count as usize, 0), // answer
        (header.ns_count as usize, 1), // authority
        (header.ar_count as usize, 2), // additional
    ];

    for (count, section_id) in sections {
        for _ in 0..count {
            if offset >= buf.len() {
                break;
            }
            let (name, new_offset) = parse_name(buf, offset)?;
            offset = new_offset;

            if offset + 10 > buf.len() {
                return Err(DnsParseError::Truncated(offset));
            }

            let rtype_raw = u16::from_be_bytes([buf[offset], buf[offset + 1]]);
            let rtype = RecordType::from_u16(rtype_raw);
            let class = RecordClass::from_u16(u16::from_be_bytes([buf[offset + 2], buf[offset + 3]]));
            let ttl = u32::from_be_bytes([buf[offset + 4], buf[offset + 5], buf[offset + 6], buf[offset + 7]]);
            let rdlength = u16::from_be_bytes([buf[offset + 8], buf[offset + 9]]) as usize;
            offset += 10;

            if offset + rdlength > buf.len() {
                return Err(DnsParseError::Truncated(offset));
            }

            let rdata = parse_rdata(buf, offset, rdlength, rtype)?;
            offset += rdlength;

            // Filter OPT records (type 41) — RFC 6891: must not be cached or forwarded
            if rtype_raw == 41 {
                continue;
            }

            let record = DnsRecord {
                name: name.to_lowercase(),
                rtype,
                class,
                ttl,
                rdata,
            };

            match section_id {
                0 => answers.push(record),
                1 => authority.push(record),
                _ => additional.push(record),
            }
        }
    }

    Ok(ParsedResponse {
        header,
        answers,
        authority,
        additional,
    })
}

fn parse_rdata(buf: &[u8], offset: usize, rdlength: usize, rtype: RecordType) -> Result<RData, DnsParseError> {
    match rtype {
        RecordType::A => {
            if rdlength != 4 {
                return Ok(RData::Raw(buf[offset..offset + rdlength].to_vec()));
            }
            Ok(RData::A(Ipv4Addr::new(
                buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3],
            )))
        }
        RecordType::AAAA => {
            if rdlength != 16 {
                return Ok(RData::Raw(buf[offset..offset + rdlength].to_vec()));
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&buf[offset..offset + 16]);
            Ok(RData::AAAA(Ipv6Addr::from(octets)))
        }
        RecordType::CNAME | RecordType::PTR | RecordType::NS => {
            let (name, _) = parse_name(buf, offset)?;
            match rtype {
                RecordType::CNAME => Ok(RData::CNAME(name)),
                RecordType::PTR => Ok(RData::PTR(name)),
                RecordType::NS => Ok(RData::NS(name)),
                _ => unreachable!(),
            }
        }
        RecordType::MX => {
            if rdlength < 3 {
                return Ok(RData::Raw(buf[offset..offset + rdlength].to_vec()));
            }
            let preference = u16::from_be_bytes([buf[offset], buf[offset + 1]]);
            let (exchange, _) = parse_name(buf, offset + 2)?;
            Ok(RData::MX { preference, exchange })
        }
        RecordType::TXT => {
            // TXT records: one or more <length><string> pairs
            let mut txt = String::new();
            let mut pos = offset;
            let end = offset + rdlength;
            while pos < end {
                let len = buf[pos] as usize;
                pos += 1;
                if pos + len > end {
                    break;
                }
                if let Ok(s) = std::str::from_utf8(&buf[pos..pos + len]) {
                    txt.push_str(s);
                }
                pos += len;
            }
            Ok(RData::TXT(txt))
        }
        RecordType::SOA => {
            let (mname, new_offset) = parse_name(buf, offset)?;
            let (rname, new_offset) = parse_name(buf, new_offset)?;
            if new_offset + 20 > buf.len() {
                return Ok(RData::Raw(buf[offset..offset + rdlength].to_vec()));
            }
            let o = new_offset;
            Ok(RData::SOA {
                mname,
                rname,
                serial: u32::from_be_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]),
                refresh: u32::from_be_bytes([buf[o + 4], buf[o + 5], buf[o + 6], buf[o + 7]]),
                retry: u32::from_be_bytes([buf[o + 8], buf[o + 9], buf[o + 10], buf[o + 11]]),
                expire: u32::from_be_bytes([buf[o + 12], buf[o + 13], buf[o + 14], buf[o + 15]]),
                minimum: u32::from_be_bytes([buf[o + 16], buf[o + 17], buf[o + 18], buf[o + 19]]),
            })
        }
        _ => Ok(RData::Raw(buf[offset..offset + rdlength].to_vec())),
    }
}

/// Build a DNS response packet from a query and answer records.
pub fn build_response(query: &DnsQuery, answers: &[DnsRecord], rcode: u8) -> Vec<u8> {
    let mut buf = Vec::with_capacity(512);

    // Header
    buf.extend_from_slice(&query.header.id.to_be_bytes());

    // Flags: QR=1, RD=query.RD, RA=1, rcode
    let flags: u16 = 0x8000 // QR = response
        | (if query.header.recursion_desired() { 0x0100 } else { 0 }) // RD
        | 0x0080 // RA
        | (rcode as u16 & 0xF);
    buf.extend_from_slice(&flags.to_be_bytes());

    // Counts
    buf.extend_from_slice(&query.header.qd_count.to_be_bytes()); // questions
    buf.extend_from_slice(&(answers.len() as u16).to_be_bytes()); // answers
    buf.extend_from_slice(&0u16.to_be_bytes()); // authority
    buf.extend_from_slice(&0u16.to_be_bytes()); // additional

    // Copy question section from original query
    buf.extend_from_slice(&query.raw_question_bytes);

    // Write answer records
    for record in answers {
        encode_name(&record.name, &mut buf);
        buf.extend_from_slice(&record.rtype.to_u16().to_be_bytes());
        buf.extend_from_slice(&record.class.to_u16().to_be_bytes());
        buf.extend_from_slice(&record.ttl.to_be_bytes());
        encode_rdata(&record.rdata, &mut buf);
    }

    buf
}

/// Build an error response (SERVFAIL, NXDOMAIN, etc.)
pub fn build_error_response(query: &DnsQuery, rcode: u8) -> Vec<u8> {
    build_response(query, &[], rcode)
}

fn encode_rdata(rdata: &RData, buf: &mut Vec<u8>) {
    match rdata {
        RData::A(ip) => {
            buf.extend_from_slice(&4u16.to_be_bytes()); // RDLENGTH
            buf.extend_from_slice(&ip.octets());
        }
        RData::AAAA(ip) => {
            buf.extend_from_slice(&16u16.to_be_bytes());
            buf.extend_from_slice(&ip.octets());
        }
        RData::CNAME(name) | RData::PTR(name) | RData::NS(name) => {
            let mut rdata_buf = Vec::new();
            encode_name(name, &mut rdata_buf);
            buf.extend_from_slice(&(rdata_buf.len() as u16).to_be_bytes());
            buf.extend_from_slice(&rdata_buf);
        }
        RData::MX { preference, exchange } => {
            let mut rdata_buf = Vec::new();
            rdata_buf.extend_from_slice(&preference.to_be_bytes());
            encode_name(exchange, &mut rdata_buf);
            buf.extend_from_slice(&(rdata_buf.len() as u16).to_be_bytes());
            buf.extend_from_slice(&rdata_buf);
        }
        RData::TXT(text) => {
            let text_bytes = text.as_bytes();
            // TXT records split into 255-byte chunks
            let mut rdata_buf = Vec::new();
            for chunk in text_bytes.chunks(255) {
                rdata_buf.push(chunk.len() as u8);
                rdata_buf.extend_from_slice(chunk);
            }
            if text_bytes.is_empty() {
                rdata_buf.push(0);
            }
            buf.extend_from_slice(&(rdata_buf.len() as u16).to_be_bytes());
            buf.extend_from_slice(&rdata_buf);
        }
        RData::SOA { mname, rname, serial, refresh, retry, expire, minimum } => {
            let mut rdata_buf = Vec::new();
            encode_name(mname, &mut rdata_buf);
            encode_name(rname, &mut rdata_buf);
            rdata_buf.extend_from_slice(&serial.to_be_bytes());
            rdata_buf.extend_from_slice(&refresh.to_be_bytes());
            rdata_buf.extend_from_slice(&retry.to_be_bytes());
            rdata_buf.extend_from_slice(&expire.to_be_bytes());
            rdata_buf.extend_from_slice(&minimum.to_be_bytes());
            buf.extend_from_slice(&(rdata_buf.len() as u16).to_be_bytes());
            buf.extend_from_slice(&rdata_buf);
        }
        RData::SRV { priority, weight, port, target } => {
            let mut rdata_buf = Vec::new();
            rdata_buf.extend_from_slice(&priority.to_be_bytes());
            rdata_buf.extend_from_slice(&weight.to_be_bytes());
            rdata_buf.extend_from_slice(&port.to_be_bytes());
            encode_name(target, &mut rdata_buf);
            buf.extend_from_slice(&(rdata_buf.len() as u16).to_be_bytes());
            buf.extend_from_slice(&rdata_buf);
        }
        RData::Raw(data) => {
            buf.extend_from_slice(&(data.len() as u16).to_be_bytes());
            buf.extend_from_slice(data);
        }
    }
}

/// Truncate a DNS response to fit within the given max UDP size.
/// Sets the TC (truncated) flag if the response exceeds the limit.
/// For UDP without EDNS0, max_size should be 512.
pub fn truncate_for_udp(response: &mut Vec<u8>, max_size: usize) {
    if response.len() <= max_size {
        return;
    }
    // Set TC flag (bit 1 of byte 2)
    if response.len() >= 3 {
        response[2] |= 0x02;
    }
    // Truncate to max_size — keep header + question, drop partial answers
    response.truncate(max_size);
    // Zero out answer/authority/additional counts since they're now incomplete
    if response.len() >= 12 {
        // Set AN, NS, AR counts to 0 (we can't guarantee partial records are valid)
        response[6] = 0;
        response[7] = 0;
        response[8] = 0;
        response[9] = 0;
        response[10] = 0;
        response[11] = 0;
    }
}

// RCODE constants
pub const RCODE_NOERROR: u8 = 0;
pub const RCODE_FORMERR: u8 = 1;
pub const RCODE_SERVFAIL: u8 = 2;
pub const RCODE_NXDOMAIN: u8 = 3;
pub const RCODE_NOTIMP: u8 = 4;
pub const RCODE_REFUSED: u8 = 5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_name() {
        let mut buf = Vec::new();
        encode_name("www.example.com", &mut buf);
        assert_eq!(buf, b"\x03www\x07example\x03com\x00");

        let (name, end) = parse_name(&buf, 0).unwrap();
        assert_eq!(name, "www.example.com");
        assert_eq!(end, buf.len());
    }

    #[test]
    fn test_encode_empty_name() {
        let mut buf = Vec::new();
        encode_name("", &mut buf);
        assert_eq!(buf, b"\x00");
    }

    #[test]
    fn test_parse_name_with_pointer() {
        // Name at offset 0: "example.com"
        let mut buf = Vec::new();
        encode_name("example.com", &mut buf);
        let ptr_offset = buf.len();
        // Name at ptr_offset: pointer to offset 0 -> "example.com"
        buf.push(0xC0);
        buf.push(0x00);

        let (name, end) = parse_name(&buf, ptr_offset).unwrap();
        assert_eq!(name, "example.com");
        assert_eq!(end, ptr_offset + 2);
    }

    #[test]
    fn test_build_and_parse_a_query() {
        // Build a query manually
        let mut query_buf: Vec<u8> = Vec::new();
        // Header: ID=0x1234, flags=0x0100 (RD=1), QD=1, AN=0, NS=0, AR=0
        query_buf.extend_from_slice(&[0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        // Question: example.com A IN
        encode_name("example.com", &mut query_buf);
        query_buf.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // A, IN

        let query = parse_query(&query_buf).unwrap();
        assert_eq!(query.header.id, 0x1234);
        assert!(query.header.recursion_desired());
        assert_eq!(query.questions.len(), 1);
        assert_eq!(query.questions[0].name, "example.com");
        assert_eq!(query.questions[0].qtype, RecordType::A);
    }

    #[test]
    fn test_build_response() {
        // Build a query
        let mut query_buf: Vec<u8> = Vec::new();
        query_buf.extend_from_slice(&[0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        encode_name("example.com", &mut query_buf);
        query_buf.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);

        let query = parse_query(&query_buf).unwrap();

        let answers = vec![DnsRecord::a("example.com", Ipv4Addr::new(93, 184, 216, 34), 300)];
        let response = build_response(&query, &answers, RCODE_NOERROR);

        // Parse the response
        let (header, records) = parse_response_records(&response).unwrap();
        assert!(header.is_response());
        assert_eq!(header.rcode(), RCODE_NOERROR);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "example.com");
        if let RData::A(ip) = &records[0].rdata {
            assert_eq!(*ip, Ipv4Addr::new(93, 184, 216, 34));
        } else {
            panic!("Expected A record");
        }
    }
}
