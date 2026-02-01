use axum::{
    body::Body,
    extract::Request,
    http::{HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use ipnet::IpNet;
use std::net::IpAddr;
use std::sync::{Arc, RwLock};
use tokio::net::TcpStream;
use tracing::{warn, error, debug, info};

use crate::auth;
use crate::config::{ProxyConfig, RouteConfig};
use crate::logging::{self, AccessLogEntry, OptionalAccessLogger};

/// Snapshot of parsed config for fast lookups
struct ConfigSnapshot {
    config: ProxyConfig,
    local_networks: Vec<IpNet>,
}

/// Shared proxy state with reloadable config
pub struct ProxyState {
    /// HTTP client for backend requests
    pub client: Client<hyper_util::client::legacy::connect::HttpConnector, Body>,
    /// Reloadable configuration snapshot
    snapshot: RwLock<ConfigSnapshot>,
    /// Access logger
    pub access_logger: OptionalAccessLogger,
}

impl ProxyState {
    pub fn new(config: ProxyConfig) -> Self {
        let client = Client::builder(TokioExecutor::new())
            .build_http();

        let access_logger = OptionalAccessLogger::new(config.access_log_path.clone());

        let local_networks: Vec<IpNet> = config
            .local_networks
            .iter()
            .filter_map(|n| n.parse().ok())
            .collect();

        Self {
            client,
            snapshot: RwLock::new(ConfigSnapshot { config, local_networks }),
            access_logger,
        }
    }

    /// Reload the proxy config (called on SIGHUP)
    pub fn reload_config(&self, new_config: ProxyConfig) {
        let local_networks: Vec<IpNet> = new_config
            .local_networks
            .iter()
            .filter_map(|n| n.parse().ok())
            .collect();

        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.config = new_config;
        snapshot.local_networks = local_networks;
    }

    /// Check if an IP address is in a local network
    pub fn is_local_ip(&self, ip: &IpAddr) -> bool {
        let snapshot = self.snapshot.read().unwrap();
        snapshot.local_networks.iter().any(|net| net.contains(ip))
    }

    /// Find the route matching a given Host header
    pub fn find_route(&self, host: &str) -> Option<RouteConfig> {
        let domain = host.split(':').next().unwrap_or(host);
        let snapshot = self.snapshot.read().unwrap();
        snapshot.config
            .routes
            .iter()
            .find(|r| r.enabled && r.domain == domain)
            .cloned()
    }

    /// Get auth service URL
    pub fn auth_service_url(&self) -> String {
        let snapshot = self.snapshot.read().unwrap();
        snapshot.config.auth_service_url.clone()
    }
}

/// Main proxy handler - dispatches by Host header
pub async fn proxy_handler(
    state: Arc<ProxyState>,
    client_ip: IpAddr,
    req: Request,
) -> Result<Response, ProxyError> {
    let start = std::time::Instant::now();

    // Extract info for logging before passing ownership
    let method = req.method().to_string();
    let path = req.uri().path_and_query().map(|pq| pq.to_string()).unwrap_or_else(|| "/".to_string());
    let user_agent = req.headers().get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let host_for_log = req.headers().get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let result = proxy_handler_inner(state.clone(), client_ip, req).await;

    // Log the request
    let status = match &result {
        Ok(resp) => resp.status().as_u16(),
        Err(e) => match e {
            ProxyError::DomainNotFound(_) => 404,
            ProxyError::Forbidden => 403,
            ProxyError::AuthRequired => 401,
            ProxyError::UpstreamError(_) => 502,
            ProxyError::InvalidUri(_) => 400,
        },
    };

    state.access_logger.log(AccessLogEntry {
        timestamp: logging::now_timestamp(),
        client_ip: client_ip.to_string(),
        host: host_for_log,
        method,
        path,
        status,
        duration_ms: start.elapsed().as_millis() as u64,
        user_agent,
    });

    result
}

/// Inner proxy handler logic
async fn proxy_handler_inner(
    state: Arc<ProxyState>,
    client_ip: IpAddr,
    mut req: Request,
) -> Result<Response, ProxyError> {
    // Extract Host header
    let host = req
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    debug!("Request from {} for host: {} {}", client_ip, host, req.uri().path());

    // Find matching route
    let route = state
        .find_route(&host)
        .ok_or(ProxyError::DomainNotFound(host.clone()))?;

    // IP filtering for localOnly routes
    if route.local_only && !state.is_local_ip(&client_ip) {
        warn!("Blocked non-local IP {} for local-only route {}", client_ip, route.domain);
        return Err(ProxyError::Forbidden);
    }

    // Forward-auth for routes requiring authentication
    if route.require_auth {
        let auth_url = state.auth_service_url();
        match auth::check_auth(&auth_url, req.headers()).await {
            Ok(auth_resp) => {
                if !auth_resp.success {
                    return Err(ProxyError::AuthRequired);
                }
                debug!("Auth OK for user: {:?}", auth_resp.user);
            }
            Err(auth::AuthError::Unauthorized) => {
                return Err(ProxyError::AuthRequired);
            }
            Err(e) => {
                error!("Auth service error: {}", e);
                return Err(ProxyError::AuthRequired);
            }
        }
    }

    // Check if this is a WebSocket upgrade request
    let is_websocket = is_websocket_upgrade(&req);

    // Build target URL
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|x| x.to_string())
        .unwrap_or_else(|| "/".to_string());

    let target_url = format!(
        "http://{}:{}{}",
        route.target_host, route.target_port, &path_and_query
    );

    let target_uri: Uri = target_url
        .parse()
        .map_err(|e| ProxyError::InvalidUri(format!("{}", e)))?;

    // Set forwarding headers
    let headers = req.headers_mut();

    // X-Forwarded-* headers
    if let Ok(val) = HeaderValue::from_str(&host) {
        headers.insert("X-Forwarded-Host", val);
    }
    headers.insert("X-Forwarded-Proto", HeaderValue::from_static("https"));
    if let Ok(val) = HeaderValue::from_str(&client_ip.to_string()) {
        headers.insert("X-Forwarded-For", val.clone());
        headers.insert("X-Real-IP", val);
    }

    if is_websocket {
        // WebSocket: use low-level hyper connection for upgrade support
        debug!("WebSocket upgrade detected for {}", host);
        let path_only: Uri = path_and_query.parse().unwrap_or_else(|_| "/".parse().unwrap());
        return handle_websocket_upgrade(req, &route, path_only).await;
    }

    // For normal HTTP: remove hop-by-hop headers
    headers.remove("connection");
    headers.remove("upgrade");

    // Update the URI
    *req.uri_mut() = target_uri;

    // Forward the request via pooled client
    let response = state
        .client
        .request(req)
        .await
        .map_err(|e| ProxyError::UpstreamError(e.to_string()))?;

    Ok(response.into_response())
}

/// Handle WebSocket upgrade by establishing a direct connection to the backend
/// and bidirectionally copying data after the upgrade handshake.
async fn handle_websocket_upgrade(
    mut req: Request,
    route: &RouteConfig,
    target_uri: Uri,
) -> Result<Response, ProxyError> {
    use hyper::client::conn::http1::Builder;
    use tokio::io::AsyncWriteExt;

    // Extract the client-side upgrade future BEFORE forwarding the request.
    // This removes the OnUpgrade extension from the request.
    let client_upgrade = hyper::upgrade::on(&mut req);

    // Connect to backend
    let backend_addr = format!("{}:{}", route.target_host, route.target_port);
    let tcp_stream = TcpStream::connect(&backend_addr)
        .await
        .map_err(|e| ProxyError::UpstreamError(format!("Failed to connect to backend {}: {}", backend_addr, e)))?;

    let io = TokioIo::new(tcp_stream);

    // Create a one-shot HTTP/1.1 connection to the backend
    let (mut sender, conn) = Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .handshake(io)
        .await
        .map_err(|e| ProxyError::UpstreamError(format!("Backend handshake failed: {}", e)))?;

    // Spawn the connection driver with upgrade support
    tokio::spawn(async move {
        if let Err(e) = conn.with_upgrades().await {
            let msg = e.to_string();
            if !msg.contains("connection closed") && !msg.contains("not connected") {
                error!("WebSocket backend connection error: {}", e);
            }
        }
    });

    // Update the request URI for the backend
    *req.uri_mut() = target_uri;

    // Send the upgrade request to the backend
    let backend_response = sender
        .send_request(req)
        .await
        .map_err(|e| ProxyError::UpstreamError(format!("Backend request failed: {}", e)))?;

    if backend_response.status() != StatusCode::SWITCHING_PROTOCOLS {
        warn!("Backend did not upgrade WebSocket, status: {}", backend_response.status());
        return Ok(backend_response.into_response());
    }

    info!("WebSocket upgrade successful to {}", backend_addr);

    // Extract relevant headers from backend 101 response before consuming it
    let mut response_builder = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS);

    // Forward all headers from the backend 101 response
    for (name, value) in backend_response.headers() {
        response_builder = response_builder.header(name, value);
    }

    // Extract the backend upgrade future (consumes the response)
    let backend_upgrade = hyper::upgrade::on(backend_response);

    let client_response = response_builder
        .body(Body::empty())
        .unwrap();

    // Spawn a task to bridge the two upgraded connections
    tokio::spawn(async move {
        match tokio::try_join!(client_upgrade, backend_upgrade) {
            Ok((client_io, backend_io)) => {
                let mut client_io = TokioIo::new(client_io);
                let mut backend_io = TokioIo::new(backend_io);
                match tokio::io::copy_bidirectional(&mut client_io, &mut backend_io).await {
                    Ok((from_client, from_backend)) => {
                        debug!("WebSocket closed: {} bytes client->backend, {} bytes backend->client", from_client, from_backend);
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if !msg.contains("connection reset") && !msg.contains("broken pipe") {
                            debug!("WebSocket IO error: {}", e);
                        }
                    }
                }
                // Ensure both sides are properly shut down
                let _ = client_io.shutdown().await;
                let _ = backend_io.shutdown().await;
            }
            Err(e) => {
                error!("WebSocket upgrade bridging failed: {}", e);
            }
        }
    });

    Ok(client_response)
}

/// Check if the request is a WebSocket upgrade
fn is_websocket_upgrade(req: &Request) -> bool {
    let has_upgrade = req
        .headers()
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    let has_connection_upgrade = req
        .headers()
        .get("connection")
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);

    has_upgrade && has_connection_upgrade
}

/// Proxy errors
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("Invalid URI: {0}")]
    InvalidUri(String),

    #[error("Upstream error: {0}")]
    UpstreamError(String),

    #[error("Authentication required")]
    AuthRequired,

    #[error("Forbidden")]
    Forbidden,

    #[error("Domain not found: {0}")]
    DomainNotFound(String),
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ProxyError::InvalidUri(msg) => (StatusCode::BAD_REQUEST, msg),
            ProxyError::UpstreamError(msg) => (StatusCode::BAD_GATEWAY, msg),
            ProxyError::AuthRequired => (StatusCode::UNAUTHORIZED, "Authentication required".to_string()),
            ProxyError::Forbidden => (StatusCode::FORBIDDEN, "Forbidden".to_string()),
            ProxyError::DomainNotFound(domain) => (StatusCode::NOT_FOUND, format!("Domain not configured: {}", domain)),
        };

        (status, message).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ProxyConfig, RouteConfig};
    use axum::http::Request;
    use std::path::PathBuf;

    fn test_config() -> ProxyConfig {
        ProxyConfig {
            http_port: 80,
            https_port: 443,
            base_domain: "example.com".to_string(),
            tls_mode: "local-ca".to_string(),
            ca_storage_path: PathBuf::from("/tmp/ca"),
            auth_service_url: "http://localhost:4000".to_string(),
            routes: vec![
                RouteConfig {
                    id: "route-1".to_string(),
                    domain: "app.example.com".to_string(),
                    backend: "rust".to_string(),
                    target_host: "localhost".to_string(),
                    target_port: 3000,
                    local_only: false,
                    require_auth: false,
                    enabled: true,
                    cert_id: Some("cert-1".to_string()),
                },
                RouteConfig {
                    id: "route-2".to_string(),
                    domain: "local.example.com".to_string(),
                    backend: "rust".to_string(),
                    target_host: "localhost".to_string(),
                    target_port: 3001,
                    local_only: true,
                    require_auth: false,
                    enabled: true,
                    cert_id: Some("cert-2".to_string()),
                },
                RouteConfig {
                    id: "route-3".to_string(),
                    domain: "auth.example.com".to_string(),
                    backend: "rust".to_string(),
                    target_host: "localhost".to_string(),
                    target_port: 3002,
                    local_only: false,
                    require_auth: true,
                    enabled: true,
                    cert_id: Some("cert-3".to_string()),
                },
                RouteConfig {
                    id: "route-4".to_string(),
                    domain: "disabled.example.com".to_string(),
                    backend: "rust".to_string(),
                    target_host: "localhost".to_string(),
                    target_port: 3003,
                    local_only: false,
                    require_auth: false,
                    enabled: false,
                    cert_id: None,
                },
                RouteConfig {
                    id: "route-5".to_string(),
                    domain: "caddy.example.com".to_string(),
                    backend: "caddy".to_string(),
                    target_host: "localhost".to_string(),
                    target_port: 3004,
                    local_only: false,
                    require_auth: false,
                    enabled: true,
                    cert_id: None,
                },
            ],
            access_log_path: None,
            local_networks: vec![
                "192.168.0.0/16".to_string(),
                "10.0.0.0/8".to_string(),
                "172.16.0.0/12".to_string(),
                "127.0.0.0/8".to_string(),
            ],
        }
    }

    #[test]
    fn test_find_route_by_domain() {
        let state = ProxyState::new(test_config());
        let route = state.find_route("app.example.com");
        assert!(route.is_some());
        assert_eq!(route.unwrap().target_port, 3000);
    }

    #[test]
    fn test_find_route_strips_port() {
        let state = ProxyState::new(test_config());
        let route = state.find_route("app.example.com:444");
        assert!(route.is_some());
        assert_eq!(route.unwrap().domain, "app.example.com");
    }

    #[test]
    fn test_find_route_unknown_domain() {
        let state = ProxyState::new(test_config());
        assert!(state.find_route("unknown.example.com").is_none());
    }

    #[test]
    fn test_find_route_disabled() {
        let state = ProxyState::new(test_config());
        assert!(state.find_route("disabled.example.com").is_none());
    }

    #[test]
    fn test_find_route_any_backend() {
        let state = ProxyState::new(test_config());
        // All backends are now served by Rust proxy
        let route = state.find_route("caddy.example.com");
        assert!(route.is_some());
        assert_eq!(route.unwrap().target_port, 3004);
    }

    #[test]
    fn test_is_local_ip_loopback() {
        let state = ProxyState::new(test_config());
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(state.is_local_ip(&ip));
    }

    #[test]
    fn test_is_local_ip_private_192() {
        let state = ProxyState::new(test_config());
        let ip: IpAddr = "192.168.1.100".parse().unwrap();
        assert!(state.is_local_ip(&ip));
    }

    #[test]
    fn test_is_local_ip_private_10() {
        let state = ProxyState::new(test_config());
        let ip: IpAddr = "10.0.0.5".parse().unwrap();
        assert!(state.is_local_ip(&ip));
    }

    #[test]
    fn test_is_local_ip_private_172() {
        let state = ProxyState::new(test_config());
        let ip: IpAddr = "172.16.5.10".parse().unwrap();
        assert!(state.is_local_ip(&ip));
    }

    #[test]
    fn test_is_not_local_ip_public() {
        let state = ProxyState::new(test_config());
        let ip: IpAddr = "8.8.8.8".parse().unwrap();
        assert!(!state.is_local_ip(&ip));
    }

    #[test]
    fn test_is_not_local_ip_172_outside_range() {
        let state = ProxyState::new(test_config());
        let ip: IpAddr = "172.32.0.1".parse().unwrap();
        assert!(!state.is_local_ip(&ip));
    }

    #[test]
    fn test_websocket_upgrade_detection() {
        let req = Request::builder()
            .header("upgrade", "websocket")
            .header("connection", "Upgrade")
            .body(Body::empty())
            .unwrap();
        assert!(is_websocket_upgrade(&req));
    }

    #[test]
    fn test_websocket_upgrade_case_insensitive() {
        let req = Request::builder()
            .header("upgrade", "WebSocket")
            .header("connection", "keep-alive, Upgrade")
            .body(Body::empty())
            .unwrap();
        assert!(is_websocket_upgrade(&req));
    }

    #[test]
    fn test_not_websocket_no_upgrade() {
        let req = Request::builder()
            .header("connection", "keep-alive")
            .body(Body::empty())
            .unwrap();
        assert!(!is_websocket_upgrade(&req));
    }

    #[test]
    fn test_not_websocket_missing_connection() {
        let req = Request::builder()
            .header("upgrade", "websocket")
            .body(Body::empty())
            .unwrap();
        assert!(!is_websocket_upgrade(&req));
    }

    #[test]
    fn test_not_websocket_wrong_upgrade() {
        let req = Request::builder()
            .header("upgrade", "h2c")
            .header("connection", "Upgrade")
            .body(Body::empty())
            .unwrap();
        assert!(!is_websocket_upgrade(&req));
    }

    #[test]
    fn test_proxy_error_status_codes() {
        use axum::response::IntoResponse;

        let err = ProxyError::DomainNotFound("test.com".to_string());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let err = ProxyError::Forbidden;
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        let err = ProxyError::AuthRequired;
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let err = ProxyError::UpstreamError("timeout".to_string());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);

        let err = ProxyError::InvalidUri("bad".to_string());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_local_networks_parsed_correctly() {
        let state = ProxyState::new(test_config());
        // Verify via is_local_ip checks rather than accessing private field
        assert!(state.is_local_ip(&"127.0.0.1".parse().unwrap()));
        assert!(state.is_local_ip(&"192.168.1.1".parse().unwrap()));
        assert!(state.is_local_ip(&"10.0.0.1".parse().unwrap()));
        assert!(state.is_local_ip(&"172.16.0.1".parse().unwrap()));
        assert!(!state.is_local_ip(&"8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn test_reload_config() {
        let mut config = test_config();
        let state = ProxyState::new(config.clone());
        assert!(state.find_route("app.example.com").is_some());
        assert!(state.find_route("new.example.com").is_none());

        // Add a new route and reload
        config.routes.push(RouteConfig {
            id: "route-new".to_string(),
            domain: "new.example.com".to_string(),
            backend: "rust".to_string(),
            target_host: "localhost".to_string(),
            target_port: 5000,
            local_only: false,
            require_auth: false,
            enabled: true,
            cert_id: None,
        });
        state.reload_config(config);

        assert!(state.find_route("new.example.com").is_some());
        assert_eq!(state.find_route("new.example.com").unwrap().target_port, 5000);
    }
}
