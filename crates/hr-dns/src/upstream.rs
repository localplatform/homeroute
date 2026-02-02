use std::net::SocketAddr;
use std::time::Duration;
use anyhow::Result;
use rand::Rng;
use tokio::net::{TcpStream, UdpSocket};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;
use tracing::debug;

pub struct UpstreamForwarder {
    servers: Vec<SocketAddr>,
    timeout_ms: u64,
}

impl UpstreamForwarder {
    pub fn new(servers: Vec<String>, timeout_ms: u64) -> Self {
        let servers: Vec<SocketAddr> = servers
            .iter()
            .filter_map(|s| {
                if s.contains(':') && !s.starts_with('[') {
                    // IPv6 without port
                    format!("[{}]:53", s).parse().ok()
                } else if s.contains("]:") {
                    // IPv6 with port
                    s.parse().ok()
                } else if s.contains(':') && s.matches(':').count() == 1 {
                    // IPv4 with port
                    s.parse().ok()
                } else {
                    // IPv4 without port
                    format!("{}:53", s).parse().ok()
                }
            })
            .collect();

        Self { servers, timeout_ms }
    }

    /// Forward a DNS query to upstream servers via UDP.
    /// Randomizes the TXID for upstream and validates the response.
    /// Falls back to TCP if response is truncated.
    pub async fn forward(&self, query_bytes: &[u8]) -> Result<Vec<u8>> {
        if query_bytes.len() < 12 {
            anyhow::bail!("Query too short to forward");
        }

        let half_timeout = Duration::from_millis(self.timeout_ms / 2);
        let full_timeout = Duration::from_millis(self.timeout_ms);

        // Generate a random TXID for the upstream query (RFC 5452)
        let original_txid = u16::from_be_bytes([query_bytes[0], query_bytes[1]]);
        let upstream_txid: u16 = rand::rng().random();

        // Build upstream query with randomized TXID
        let mut upstream_query = query_bytes.to_vec();
        upstream_query[0] = (upstream_txid >> 8) as u8;
        upstream_query[1] = (upstream_txid & 0xFF) as u8;

        // Try UDP first
        for (i, server) in self.servers.iter().enumerate() {
            let t = if i == 0 { half_timeout } else { full_timeout };

            match self.forward_udp(&upstream_query, *server, t, upstream_txid).await {
                Ok(mut response) => {
                    // Check TC (truncated) flag
                    if response.len() >= 4 && response[2] & 0x02 != 0 {
                        debug!("Response truncated from {}, retrying TCP", server);
                        if let Ok(mut tcp_response) = self.forward_tcp(&upstream_query, *server, full_timeout).await {
                            // Restore original client TXID
                            if tcp_response.len() >= 2 {
                                tcp_response[0] = (original_txid >> 8) as u8;
                                tcp_response[1] = (original_txid & 0xFF) as u8;
                            }
                            return Ok(tcp_response);
                        }
                    }
                    // Restore original client TXID in the response
                    if response.len() >= 2 {
                        response[0] = (original_txid >> 8) as u8;
                        response[1] = (original_txid & 0xFF) as u8;
                    }
                    return Ok(response);
                }
                Err(e) => {
                    debug!("UDP forward to {} failed: {}", server, e);
                    continue;
                }
            }
        }

        anyhow::bail!("All upstream servers failed")
    }

    async fn forward_udp(
        &self,
        query: &[u8],
        server: SocketAddr,
        dur: Duration,
        expected_txid: u16,
    ) -> Result<Vec<u8>> {
        let bind_addr: SocketAddr = if server.is_ipv4() {
            "0.0.0.0:0".parse().unwrap()
        } else {
            "[::]:0".parse().unwrap()
        };

        let socket = UdpSocket::bind(bind_addr).await?;
        socket.send_to(query, server).await?;

        let mut buf = vec![0u8; 4096];
        // Use recv_from to validate source IP (RFC 5452)
        let (len, src) = timeout(dur, socket.recv_from(&mut buf)).await??;
        buf.truncate(len);

        // Validate source address matches the upstream server we queried
        if src.ip() != server.ip() {
            anyhow::bail!(
                "Response from unexpected source {} (expected {})",
                src.ip(),
                server.ip()
            );
        }

        // Validate TXID matches (RFC 5452 - cache poisoning prevention)
        if buf.len() < 12 {
            anyhow::bail!("Response too short: {} bytes", buf.len());
        }
        let response_txid = u16::from_be_bytes([buf[0], buf[1]]);
        if response_txid != expected_txid {
            anyhow::bail!(
                "TXID mismatch: expected {:04x}, got {:04x}",
                expected_txid,
                response_txid
            );
        }

        // Validate QR bit is set (this is a response)
        if buf[2] & 0x80 == 0 {
            anyhow::bail!("Response missing QR flag");
        }

        Ok(buf)
    }

    async fn forward_tcp(&self, query: &[u8], server: SocketAddr, dur: Duration) -> Result<Vec<u8>> {
        let mut stream = timeout(dur, TcpStream::connect(server)).await??;

        // TCP DNS: 2-byte length prefix
        let len_bytes = (query.len() as u16).to_be_bytes();
        stream.write_all(&len_bytes).await?;
        stream.write_all(query).await?;

        // Read response
        let mut len_buf = [0u8; 2];
        timeout(dur, stream.read_exact(&mut len_buf)).await??;
        let response_len = u16::from_be_bytes(len_buf) as usize;

        if response_len > 65535 {
            anyhow::bail!("TCP response too large: {}", response_len);
        }

        let mut response = vec![0u8; response_len];
        timeout(dur, stream.read_exact(&mut response)).await??;
        Ok(response)
    }

    pub fn update_servers(&mut self, servers: Vec<String>, timeout_ms: u64) {
        *self = Self::new(servers, timeout_ms);
    }
}
