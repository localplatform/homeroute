use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Header sent at the beginning of each QUIC stream (VPS -> on-prem).
/// Binary format: [version:u8][ip_type:u8][ip_bytes:4or16][timestamp:u64]
#[derive(Debug, Clone)]
pub struct StreamHeader {
    pub client_ip: IpAddr,
    pub timestamp: u64,
}

impl StreamHeader {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::new();
        buf.put_u8(1); // version
        match self.client_ip {
            IpAddr::V4(ip) => {
                buf.put_u8(4);
                buf.put_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                buf.put_u8(6);
                buf.put_slice(&ip.octets());
            }
        }
        buf.put_u64(self.timestamp);
        buf.freeze()
    }

    pub fn decode(buf: &mut impl Buf) -> anyhow::Result<Self> {
        anyhow::ensure!(buf.remaining() >= 2, "StreamHeader too short");
        let version = buf.get_u8();
        anyhow::ensure!(version == 1, "Unsupported StreamHeader version {}", version);
        let ip_type = buf.get_u8();
        let client_ip = match ip_type {
            4 => {
                anyhow::ensure!(buf.remaining() >= 4, "Incomplete IPv4");
                let mut octets = [0u8; 4];
                buf.copy_to_slice(&mut octets);
                IpAddr::V4(octets.into())
            }
            6 => {
                anyhow::ensure!(buf.remaining() >= 16, "Incomplete IPv6");
                let mut octets = [0u8; 16];
                buf.copy_to_slice(&mut octets);
                IpAddr::V6(octets.into())
            }
            other => anyhow::bail!("Invalid IP type: {}", other),
        };
        anyhow::ensure!(buf.remaining() >= 8, "Incomplete timestamp");
        let timestamp = buf.get_u64();
        Ok(Self { client_ip, timestamp })
    }
}

/// Control messages exchanged on a dedicated QUIC stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlMessage {
    Ping { ts: u64 },
    Pong { ts: u64, latency_us: u64 },
    RelayStats { active_streams: u32, total_bytes: u64 },
    Shutdown { reason: String },
}

impl ControlMessage {
    /// Encode a control message as length-prefixed JSON (u32 BE length + JSON bytes).
    pub fn encode(&self) -> anyhow::Result<Bytes> {
        let json = serde_json::to_vec(self)?;
        let mut buf = BytesMut::with_capacity(4 + json.len());
        buf.put_u32(json.len() as u32);
        buf.put_slice(&json);
        Ok(buf.freeze())
    }

    /// Decode a control message from length-prefixed JSON.
    pub fn decode(buf: &mut impl Buf) -> anyhow::Result<Self> {
        anyhow::ensure!(buf.remaining() >= 4, "ControlMessage: missing length prefix");
        let len = buf.get_u32() as usize;
        anyhow::ensure!(
            buf.remaining() >= len,
            "ControlMessage: expected {} bytes, got {}",
            len,
            buf.remaining()
        );
        let mut json_buf = vec![0u8; len];
        buf.copy_to_slice(&mut json_buf);
        let msg: ControlMessage = serde_json::from_slice(&json_buf)?;
        Ok(msg)
    }
}
