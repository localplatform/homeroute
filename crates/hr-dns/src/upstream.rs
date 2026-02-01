use std::net::SocketAddr;
use std::time::Duration;
use anyhow::Result;
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
    /// Tries servers in order, returns first response.
    /// Falls back to TCP if response is truncated.
    pub async fn forward(&self, query_bytes: &[u8]) -> Result<Vec<u8>> {
        let half_timeout = Duration::from_millis(self.timeout_ms / 2);
        let full_timeout = Duration::from_millis(self.timeout_ms);

        // Try UDP first
        for (i, server) in self.servers.iter().enumerate() {
            let t = if i == 0 { half_timeout } else { full_timeout };

            match self.forward_udp(query_bytes, *server, t).await {
                Ok(response) => {
                    // Check TC (truncated) flag
                    if response.len() >= 4 && response[2] & 0x02 != 0 {
                        debug!("Response truncated from {}, retrying TCP", server);
                        if let Ok(tcp_response) = self.forward_tcp(query_bytes, *server, full_timeout).await {
                            return Ok(tcp_response);
                        }
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

    async fn forward_udp(&self, query: &[u8], server: SocketAddr, dur: Duration) -> Result<Vec<u8>> {
        let bind_addr: SocketAddr = if server.is_ipv4() {
            "0.0.0.0:0".parse().unwrap()
        } else {
            "[::]:0".parse().unwrap()
        };

        let socket = UdpSocket::bind(bind_addr).await?;
        socket.send_to(query, server).await?;

        let mut buf = vec![0u8; 4096];
        let len = timeout(dur, socket.recv(&mut buf)).await??;
        buf.truncate(len);
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
