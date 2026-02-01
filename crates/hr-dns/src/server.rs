use std::net::SocketAddr;
use std::sync::Arc;
use anyhow::Result;
use tokio::net::{TcpListener, UdpSocket};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, info, warn};

use crate::SharedDnsState;
use crate::packet::{self, RCODE_FORMERR};
use crate::resolver;

/// Run a DNS UDP server on the given address.
pub async fn run_udp_server(addr: SocketAddr, state: SharedDnsState) -> Result<()> {
    let socket = Arc::new(UdpSocket::bind(addr).await?);
    info!("DNS UDP server listening on {}", addr);

    let mut buf = [0u8; 4096];

    loop {
        let (len, src) = match socket.recv_from(&mut buf).await {
            Ok(r) => r,
            Err(e) => {
                warn!("UDP recv error: {}", e);
                continue;
            }
        };

        let packet = buf[..len].to_vec();
        let socket = socket.clone();
        let state = state.clone();

        tokio::spawn(async move {
            let response = handle_dns_query(&packet, &state, src).await;
            if let Err(e) = socket.send_to(&response, src).await {
                debug!("Failed to send UDP response to {}: {}", src, e);
            }
        });
    }
}

/// Run a DNS TCP server on the given address.
pub async fn run_tcp_server(addr: SocketAddr, state: SharedDnsState) -> Result<()> {
    let listener = TcpListener::bind(addr).await?;
    info!("DNS TCP server listening on {}", addr);

    loop {
        let (stream, src) = match listener.accept().await {
            Ok(r) => r,
            Err(e) => {
                warn!("TCP accept error: {}", e);
                continue;
            }
        };

        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_tcp_connection(stream, src, &state).await {
                debug!("TCP connection error from {}: {}", src, e);
            }
        });
    }
}

async fn handle_tcp_connection(
    mut stream: tokio::net::TcpStream,
    src: SocketAddr,
    state: &SharedDnsState,
) -> Result<()> {
    // TCP DNS: read 2-byte length prefix, then message
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await?;
    let msg_len = u16::from_be_bytes(len_buf) as usize;

    if msg_len > 65535 || msg_len < 12 {
        return Ok(());
    }

    let mut query_buf = vec![0u8; msg_len];
    stream.read_exact(&mut query_buf).await?;

    let response = handle_dns_query(&query_buf, state, src).await;

    // Write response with length prefix
    let len_bytes = (response.len() as u16).to_be_bytes();
    stream.write_all(&len_bytes).await?;
    stream.write_all(&response).await?;

    Ok(())
}

async fn handle_dns_query(query_bytes: &[u8], state: &SharedDnsState, src: SocketAddr) -> Vec<u8> {
    // Parse query
    let query = match packet::parse_query(query_bytes) {
        Ok(q) => q,
        Err(e) => {
            debug!("Failed to parse DNS query from {}: {}", src, e);
            // Return FORMERR if we can parse at least the header
            if query_bytes.len() >= 12 {
                let mut err_resp = query_bytes[..12].to_vec();
                // Set QR=1, RCODE=FORMERR
                err_resp[2] |= 0x80;
                err_resp[3] = (err_resp[3] & 0xF0) | RCODE_FORMERR;
                return err_resp;
            }
            return vec![];
        }
    };

    let start = std::time::Instant::now();

    // Resolve
    let result = resolver::resolve(&query, state).await;
    let elapsed_ms = start.elapsed().as_millis() as u64;

    // Build response
    let response = packet::build_response(&query, &result.records, result.rcode);

    // Log query and emit analytics event
    if !query.questions.is_empty() {
        let q = &query.questions[0];
        let state_read = state.read().await;
        if let Some(ref logger) = state_read.query_logger {
            logger.log(
                &q.name,
                &q.qtype.to_string(),
                &src.ip().to_string(),
                result.blocked,
                result.cached,
                elapsed_ms,
            );
        }

        // Broadcast DNS event for analytics
        if let Some(ref sender) = state_read.dns_events {
            let _ = sender.send(hr_common::events::DnsTrafficEvent {
                timestamp: chrono::Utc::now().to_rfc3339(),
                client_ip: src.ip().to_string(),
                domain: q.name.clone(),
                query_type: q.qtype.to_string(),
                blocked: result.blocked,
                cached: result.cached,
                response_time_ms: elapsed_ms,
            });
        }
    }

    response
}
