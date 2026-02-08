use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use hr_tunnel::protocol::StreamHeader;
use quinn::Connection;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Shared state: the active QUIC connection from on-prem (if any).
pub type ActiveConnection = Arc<RwLock<Option<Connection>>>;

/// Accept incoming TCP connections on the relay port and forward them through the QUIC tunnel.
pub async fn run_tcp_relay(listener: TcpListener, active_conn: ActiveConnection) -> Result<()> {
    info!("TCP relay listening on {}", listener.local_addr()?);

    loop {
        let (tcp_stream, peer_addr) = match listener.accept().await {
            Ok(r) => r,
            Err(e) => {
                warn!("TCP accept error: {}", e);
                continue;
            }
        };

        let conn = active_conn.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_tcp_connection(tcp_stream, peer_addr, conn).await {
                debug!("Relay connection from {} error: {}", peer_addr, e);
            }
        });
    }
}

async fn handle_tcp_connection(
    mut tcp_stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    active_conn: ActiveConnection,
) -> Result<()> {
    // Get the active QUIC connection (fail if not connected)
    let conn = active_conn
        .read()
        .await
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No active tunnel connection"))?
        .clone();

    // Open a bidirectional QUIC stream
    let (mut quic_send, mut quic_recv) = conn.open_bi().await?;

    // Send StreamHeader with peer IP and current timestamp
    let header = StreamHeader {
        client_ip: peer_addr.ip(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
    };
    quic_send.write_all(&header.encode()).await?;

    // Bidirectional copy between TCP and QUIC
    let (mut tcp_read, mut tcp_write) = tcp_stream.split();

    let client_to_server = tokio::io::copy(&mut tcp_read, &mut quic_send);
    let server_to_client = tokio::io::copy(&mut quic_recv, &mut tcp_write);

    tokio::select! {
        result = client_to_server => {
            if let Err(e) = result {
                debug!("TCP->QUIC copy error: {}", e);
            }
            let _ = quic_send.finish();
        }
        result = server_to_client => {
            if let Err(e) = result {
                debug!("QUIC->TCP copy error: {}", e);
            }
        }
    }

    Ok(())
}

/// Simple HTTP server that redirects all requests to HTTPS.
pub async fn run_http_redirect(port: u16) -> Result<()> {
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;

    let addr: SocketAddr = format!("[::]:{}", port).parse()?;
    let listener = TcpListener::bind(addr).await?;

    info!("HTTP redirect listening on {}", addr);

    loop {
        let (stream, _remote) = match listener.accept().await {
            Ok(r) => r,
            Err(e) => {
                warn!("HTTP redirect accept error: {}", e);
                continue;
            }
        };

        let io = TokioIo::new(stream);

        tokio::spawn(async move {
            let service = service_fn(|req: hyper::Request<hyper::body::Incoming>| async move {
                let host = req
                    .headers()
                    .get("host")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("localhost");
                let path = req
                    .uri()
                    .path_and_query()
                    .map(|pq| pq.as_str())
                    .unwrap_or("/");
                let location = format!("https://{}{}", host, path);

                Ok::<_, std::convert::Infallible>(
                    hyper::Response::builder()
                        .status(301)
                        .header("Location", &location)
                        .body(http_body_util::Empty::<hyper::body::Bytes>::new())
                        .unwrap(),
                )
            });

            if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                let msg = e.to_string();
                if !msg.contains("connection closed") && !msg.contains("not connected") {
                    debug!("HTTP redirect error: {}", msg);
                }
            }
        });
    }
}

/// Handle the control stream for a tunnel connection (ping/pong latency measurement).
pub async fn handle_control_stream(conn: &Connection) {
    loop {
        match conn.accept_uni().await {
            Ok(mut recv) => {
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    loop {
                        match recv.read(&mut buf).await {
                            Ok(Some(n)) => {
                                if let Ok(msg) = serde_json::from_slice::<serde_json::Value>(
                                    &buf[..n],
                                ) {
                                    if msg.get("type").and_then(|v| v.as_str()) == Some("ping") {
                                        debug!("Received ping from tunnel");
                                    }
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                debug!("Control stream read error: {}", e);
                                break;
                            }
                        }
                    }
                });
            }
            Err(e) => {
                debug!("Accept uni stream error: {}", e);
                break;
            }
        }
    }
}
