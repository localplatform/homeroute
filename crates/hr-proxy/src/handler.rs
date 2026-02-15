use axum::{
    body::Body,
    extract::Request,
    http::{HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use hr_auth::forward_auth::{check_forward_auth, ForwardAuthResult};
use hr_auth::AuthService;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use std::net::IpAddr;
use std::sync::{Arc, RwLock};
use tokio::net::TcpStream;
use tracing::{debug, error, info, warn};

use hr_common::events::{EventBus, HostPowerState};
use hr_registry::protocol::{ServiceAction, ServiceType};
use hr_registry::AgentRegistry;

use crate::config::{ProxyConfig, RouteConfig};
use crate::logging::{self, AccessLogEntry, OptionalAccessLogger};

/// Route to an agent-managed application (LXC container).
#[derive(Debug, Clone)]
pub struct AppRoute {
    pub app_id: String,
    pub host_id: String,
    pub target_ip: std::net::Ipv4Addr,
    pub target_port: u16,
    pub auth_required: bool,
    pub allowed_groups: Vec<String>,
    pub service_type: ServiceType,
    pub wake_page_enabled: bool,
    pub local_only: bool,
}

/// Snapshot of parsed config for fast lookups
struct ConfigSnapshot {
    config: ProxyConfig,
}

/// Shared proxy state with reloadable config
pub struct ProxyState {
    /// HTTP client for backend requests
    pub client: Client<hyper_util::client::legacy::connect::HttpConnector, Body>,
    /// HTTPS client for re-encrypt backend requests (skips cert verification for trusted LAN)
    pub https_client: reqwest::Client,
    /// Reloadable configuration snapshot
    snapshot: RwLock<ConfigSnapshot>,
    /// Access logger
    pub access_logger: OptionalAccessLogger,
    /// Auth service (direct call, no HTTP round-trip)
    pub auth: Option<Arc<AuthService>>,
    /// Management API port for proxy.{base_domain} and auth.{base_domain}
    pub management_port: u16,
    /// Application routes: domain → AppRoute (agent-managed LXC containers).
    app_routes: RwLock<std::collections::HashMap<String, AppRoute>>,
    /// Agent registry for ActivityPing and Wake-on-Demand.
    registry: RwLock<Option<Arc<AgentRegistry>>>,
    /// Event bus for service command notifications (WOD transparent wait).
    events: RwLock<Option<Arc<EventBus>>>,
}

impl ProxyState {
    pub fn new(config: ProxyConfig, management_port: u16) -> Self {
        let client = Client::builder(TokioExecutor::new()).build_http();

        // HTTPS client for re-encrypt to agent backends on port 443.
        // Skip certificate verification: agents are on a trusted LAN.
        let https_client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("Failed to build HTTPS client");

        let access_logger = OptionalAccessLogger::new(config.access_log_path.clone());

        Self {
            client,
            https_client,
            snapshot: RwLock::new(ConfigSnapshot {
                config,
            }),
            access_logger,
            auth: None,
            management_port,
            app_routes: RwLock::new(std::collections::HashMap::new()),
            registry: RwLock::new(None),
            events: RwLock::new(None),
        }
    }

    /// Set the auth service for forward-auth
    pub fn with_auth(mut self, auth: Arc<AuthService>) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Set the agent registry for ActivityPing and Wake-on-Demand.
    pub fn set_registry(&self, registry: Arc<AgentRegistry>) {
        *self.registry.write().unwrap() = Some(registry);
    }

    /// Get a clone of the registry reference.
    fn get_registry(&self) -> Option<Arc<AgentRegistry>> {
        self.registry.read().unwrap().clone()
    }

    /// Set the event bus for WOD transparent wait.
    pub fn set_events(&self, events: Arc<EventBus>) {
        *self.events.write().unwrap() = Some(events);
    }

    /// Get a clone of the event bus reference.
    fn get_events(&self) -> Option<Arc<EventBus>> {
        self.events.read().unwrap().clone()
    }

    /// Reload the proxy config (called on SIGHUP)
    pub fn reload_config(&self, new_config: ProxyConfig) {
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.config = new_config;
    }

    /// Find the route matching a given Host header
    pub fn find_route(&self, host: &str) -> Option<RouteConfig> {
        let domain = host.split(':').next().unwrap_or(host);
        let snapshot = self.snapshot.read().unwrap();
        snapshot
            .config
            .routes
            .iter()
            .find(|r| r.enabled && r.domain == domain)
            .cloned()
    }

    /// Get the base domain
    pub fn base_domain(&self) -> String {
        let snapshot = self.snapshot.read().unwrap();
        snapshot.config.base_domain.clone()
    }

    /// Get a clone of the current proxy config
    pub fn config(&self) -> ProxyConfig {
        let snapshot = self.snapshot.read().unwrap();
        snapshot.config.clone()
    }

    /// Add an application route: domain → AppRoute
    pub fn set_app_route(&self, domain: String, route: AppRoute) {
        let mut map = self.app_routes.write().unwrap();
        info!(domain = domain, target = %route.target_ip, port = route.target_port, "Added app route");
        map.insert(domain, route);
    }

    /// Remove an application route by domain.
    pub fn remove_app_route(&self, domain: &str) {
        let mut map = self.app_routes.write().unwrap();
        if map.remove(domain).is_some() {
            info!(domain = domain, "Removed app route");
        }
    }

    /// Look up an application route for a given domain.
    pub fn get_app_route(&self, domain: &str) -> Option<AppRoute> {
        let map = self.app_routes.read().unwrap();
        map.get(domain).cloned()
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
    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.to_string())
        .unwrap_or_else(|| "/".to_string());
    let user_agent = req
        .headers()
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let host_for_log = req
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let result = proxy_handler_inner(state.clone(), client_ip, req).await;

    let status = match &result {
        Ok(resp) => resp.status().as_u16(),
        Err(e) => match e {
            ProxyError::DomainNotFound(_) => 404,
            ProxyError::Forbidden => 403,
            ProxyError::AuthRequired(_) => 302,
            ProxyError::UpstreamError(_) => 502,
            ProxyError::InvalidUri(_) => 400,
        },
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    // Log to file
    state.access_logger.log(AccessLogEntry {
        timestamp: logging::now_timestamp(),
        client_ip: client_ip.to_string(),
        host: host_for_log,
        method,
        path,
        status,
        duration_ms,
        user_agent,
    });

    // Clear Alt-Svc to prevent QUIC/h3 errors in LAN — Cloudflare advertises
    // h3 support but our proxy only speaks h1/h2, so cached Alt-Svc entries
    // cause ERR_QUIC_PROTOCOL_ERROR when clients switch from WAN to LAN.
    match result {
        Ok(mut resp) => {
            resp.headers_mut()
                .insert("alt-svc", HeaderValue::from_static("clear"));
            Ok(resp)
        }
        err => err,
    }
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

    debug!(
        "Request from {} for host: {} {}",
        client_ip,
        host,
        req.uri().path()
    );

    // Built-in routes for management domains (proxy.* and auth.*)
    let base_domain = state.base_domain();
    let domain_only = host.split(':').next().unwrap_or(&host);
    let is_management = domain_only == format!("proxy.{}", base_domain)
        || domain_only == format!("auth.{}", base_domain);

    // Check for agent-managed application routes (before static route lookup)
    if !is_management {
        if let Some(app_route) = state.get_app_route(domain_only) {
            // Block ALL traffic for local-only apps
            if app_route.local_only {
                warn!("Blocked request for local-only app {} from {}", domain_only, client_ip);
                return Err(ProxyError::Forbidden);
            }

            // Agent routes (target_port == 443) handle their own auth — skip forward-auth.
            // Non-agent routes still need central forward-auth.
            let is_agent_route = app_route.target_port == 443;

            if !is_agent_route && app_route.auth_required {
                if let Some(ref auth) = state.auth {
                    let req_uri = req
                        .uri()
                        .path_and_query()
                        .map(|pq| pq.to_string())
                        .unwrap_or_else(|| "/".to_string());

                    let cookie_value = req
                        .headers()
                        .get("cookie")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|cookies| {
                            cookies
                                .split(';')
                                .find_map(|c| c.trim().strip_prefix("auth_session="))
                        });

                    match check_forward_auth(
                        auth,
                        cookie_value,
                        domain_only,
                        &req_uri,
                        "https",
                        &app_route.allowed_groups,
                    ) {
                        ForwardAuthResult::Success { user } => {
                            if let Ok(v) = HeaderValue::from_str(&user.username) {
                                req.headers_mut().insert("X-Forwarded-User", v);
                            }
                            if let Ok(v) = HeaderValue::from_str(&user.groups.join(",")) {
                                req.headers_mut().insert("X-Forwarded-Groups", v);
                            }
                        }
                        ForwardAuthResult::Unauthorized { login_url } => {
                            return Err(ProxyError::AuthRequired(Some(login_url)));
                        }
                        ForwardAuthResult::Forbidden { message } => {
                            warn!("App route auth forbidden for {}: {}", host, message);
                            return Err(ProxyError::Forbidden);
                        }
                    }
                }
            }

            // SSE endpoint for Wake-on-Demand status updates
            if req.uri().path() == "/__hr/wod" {
                return handle_wod_sse(&state, &app_route).await;
            }

            // Determine scheme based on target port (re-encrypt for agent port 443)
            let scheme = if app_route.target_port == 443 { "https" } else { "http" };

            let target_host_for_url = app_route.target_ip.to_string();

            // Check for WebSocket upgrade
            let is_websocket = is_websocket_upgrade(&req);

            if is_websocket {
                debug!("WebSocket upgrade detected for app route {}", host);
                let target_route = RouteConfig {
                    id: app_route.app_id.clone(),
                    domain: domain_only.to_string(),
                    backend: "app".to_string(),
                    target_host: target_host_for_url.clone(),
                    target_port: app_route.target_port,
                    local_only: false,
                    require_auth: false,
                    enabled: true,
                    cert_id: None,
                };
                let path_and_query = req
                    .uri()
                    .path_and_query()
                    .map(|x| x.to_string())
                    .unwrap_or_else(|| "/".to_string());
                let path_uri: Uri = path_and_query
                    .parse()
                    .unwrap_or_else(|_| "/".parse().unwrap());
                let ws_result = if is_agent_route {
                    handle_websocket_upgrade_tls(req, &target_route, path_uri, &host).await
                } else {
                    handle_websocket_upgrade(req, &target_route, path_uri).await
                };
                match ws_result {
                    Ok(resp) => return Ok(resp),
                    Err(ProxyError::UpstreamError(ref e)) if is_connection_refused(e) => {
                        // WOD: wake host or start service on connection refused
                        return Ok(handle_wod(&state, &app_route, &host).await);
                    }
                    Err(e) => return Err(e),
                }
            }

            // Regular HTTP proxy to container
            let path = req
                .uri()
                .path_and_query()
                .map(|pq| pq.to_string())
                .unwrap_or_else(|| "/".to_string());
            let target_uri_str = format!(
                "{}://{}:{}{}",
                scheme, target_host_for_url, app_route.target_port, path
            );

            // Forward headers
            let headers = req.headers_mut();
            if let Ok(v) = HeaderValue::from_str(&host) {
                headers.insert("X-Forwarded-Host", v);
            }
            if let Ok(v) = HeaderValue::from_str(&client_ip.to_string()) {
                headers.insert("X-Forwarded-For", v);
            }
            headers.insert("X-Forwarded-Proto", HeaderValue::from_static("https"));

            // Remove hop-by-hop headers
            headers.remove("connection");
            headers.remove("upgrade");

            // For HTTPS backends (re-encrypt), use reqwest with domain URL + IP resolve.
            // We use the original domain in the URL (for correct SNI) but resolve it
            // to the agent's IP address to avoid DNS lookups that may return Cloudflare.
            let proxy_result = if is_agent_route {
                let domain_uri = format!(
                    "https://{}:443{}",
                    domain_only,
                    req.uri().path_and_query().map(|pq| pq.to_string()).unwrap_or_else(|| "/".to_string())
                );
                let agent_addr = std::net::SocketAddr::new(
                    std::net::IpAddr::V4(app_route.target_ip),
                    443,
                );
                let agent_client = reqwest::Client::builder()
                    .danger_accept_invalid_certs(true)
                    .resolve(domain_only, agent_addr)
                    .build()
                    .unwrap_or_else(|_| state.https_client.clone());
                proxy_via_reqwest(&agent_client, req, &domain_uri, &host).await
            } else {
                let uri: Uri = target_uri_str
                    .parse()
                    .map_err(|e| ProxyError::InvalidUri(format!("{}", e)))?;
                *req.uri_mut() = uri;
                state
                    .client
                    .request(req)
                    .await
                    .map(|r| r.into_response())
                    .map_err(|e| e.to_string())
            };

            match proxy_result {
                Ok(resp) => {
                    // ActivityPing: notify agent of activity for powersave tracking
                    if let Some(registry) = state.get_registry() {
                        let app_id = app_route.app_id.clone();
                        let svc = app_route.service_type;
                        tokio::spawn(async move {
                            registry.send_activity_ping(&app_id, svc).await;
                        });
                    }
                    return Ok(resp);
                }
                Err(err_str) => {
                    // Wake-on-Demand: if connection refused, wake host or start service
                    if is_connection_refused(&err_str) {
                        return Ok(handle_wod(&state, &app_route, &host).await);
                    }
                    warn!("App route proxy error for {}: {}", host, err_str);
                    return Err(ProxyError::UpstreamError(err_str));
                }
            }
        }
    }

    let route = if is_management {
        RouteConfig {
            id: "__management__".to_string(),
            domain: domain_only.to_string(),
            backend: "rust".to_string(),
            target_host: "localhost".to_string(),
            target_port: state.management_port,
            local_only: false,
            require_auth: false,
            enabled: true,
            cert_id: None,
        }
    } else {
        // Find matching route
        state
            .find_route(&host)
            .ok_or(ProxyError::DomainNotFound(host.clone()))?
    };

    // Block ALL traffic for local-only routes
    if route.local_only {
        warn!("Blocked request for local-only route {} from {}", route.domain, client_ip);
        return Err(ProxyError::Forbidden);
    }

    // Forward-auth for routes requiring authentication (direct call, no HTTP)
    if route.require_auth {
        if let Some(ref auth) = state.auth {
            let req_uri = req
                .uri()
                .path_and_query()
                .map(|pq| pq.to_string())
                .unwrap_or_else(|| "/".to_string());

            // Extract auth_session cookie
            let cookie_value = req
                .headers()
                .get("cookie")
                .and_then(|v| v.to_str().ok())
                .and_then(|cookies| {
                    cookies.split(';').find_map(|c| {
                        let c = c.trim();
                        c.strip_prefix("auth_session=")
                    })
                });

            match check_forward_auth(auth, cookie_value, &host, &req_uri, "https", &[]) {
                ForwardAuthResult::Success { user } => {
                    debug!("Auth OK for user: {}", user.username);
                }
                ForwardAuthResult::Unauthorized { login_url } => {
                    return Err(ProxyError::AuthRequired(Some(login_url)));
                }
                ForwardAuthResult::Forbidden { message } => {
                    warn!("Auth forbidden: {}", message);
                    return Err(ProxyError::Forbidden);
                }
            }
        } else {
            // No auth service configured but route requires auth
            warn!("Route {} requires auth but no auth service configured", route.domain);
            return Err(ProxyError::AuthRequired(None));
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

    if let Ok(val) = HeaderValue::from_str(&host) {
        headers.insert("X-Forwarded-Host", val);
    }
    headers.insert("X-Forwarded-Proto", HeaderValue::from_static("https"));
    if let Ok(val) = HeaderValue::from_str(&client_ip.to_string()) {
        headers.insert("X-Forwarded-For", val.clone());
        headers.insert("X-Real-IP", val);
    }

    if is_websocket {
        debug!("WebSocket upgrade detected for {}", host);
        let path_only: Uri = path_and_query
            .parse()
            .unwrap_or_else(|_| "/".parse().unwrap());
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
async fn handle_websocket_upgrade(
    mut req: Request,
    route: &RouteConfig,
    target_uri: Uri,
) -> Result<Response, ProxyError> {
    use hyper::client::conn::http1::Builder;
    use tokio::io::AsyncWriteExt;

    let client_upgrade = hyper::upgrade::on(&mut req);

    let backend_addr = format!("{}:{}", route.target_host, route.target_port);
    let tcp_stream = TcpStream::connect(&backend_addr)
        .await
        .map_err(|e| {
            ProxyError::UpstreamError(format!(
                "Failed to connect to backend {}: {}",
                backend_addr, e
            ))
        })?;

    let io = TokioIo::new(tcp_stream);

    let (mut sender, conn) = Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .handshake(io)
        .await
        .map_err(|e| ProxyError::UpstreamError(format!("Backend handshake failed: {}", e)))?;

    tokio::spawn(async move {
        if let Err(e) = conn.with_upgrades().await {
            let msg = e.to_string();
            if !msg.contains("connection closed") && !msg.contains("not connected") {
                error!("WebSocket backend connection error: {}", e);
            }
        }
    });

    *req.uri_mut() = target_uri;

    let backend_response = sender
        .send_request(req)
        .await
        .map_err(|e| ProxyError::UpstreamError(format!("Backend request failed: {}", e)))?;

    if backend_response.status() != StatusCode::SWITCHING_PROTOCOLS {
        warn!(
            "Backend did not upgrade WebSocket, status: {}",
            backend_response.status()
        );
        return Ok(backend_response.into_response());
    }

    info!("WebSocket upgrade successful to {}", backend_addr);

    let mut response_builder = Response::builder().status(StatusCode::SWITCHING_PROTOCOLS);

    for (name, value) in backend_response.headers() {
        response_builder = response_builder.header(name, value);
    }

    let backend_upgrade = hyper::upgrade::on(backend_response);

    let client_response = response_builder.body(Body::empty()).unwrap();

    tokio::spawn(async move {
        match tokio::try_join!(client_upgrade, backend_upgrade) {
            Ok((client_io, backend_io)) => {
                let mut client_io = TokioIo::new(client_io);
                let mut backend_io = TokioIo::new(backend_io);
                match tokio::io::copy_bidirectional(&mut client_io, &mut backend_io).await {
                    Ok((from_client, from_backend)) => {
                        debug!(
                            "WebSocket closed: {} bytes client->backend, {} bytes backend->client",
                            from_client, from_backend
                        );
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if !msg.contains("connection reset") && !msg.contains("broken pipe") {
                            debug!("WebSocket IO error: {}", e);
                        }
                    }
                }
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

/// Proxy an HTTP request via reqwest (used for HTTPS re-encrypt backends).
/// Converts the axum Request into a reqwest Request, forwards it, and
/// converts the reqwest Response back into an axum Response.
async fn proxy_via_reqwest(
    client: &reqwest::Client,
    req: Request,
    target_url: &str,
    original_host: &str,
) -> Result<Response, String> {
    let method = req.method().clone();
    let headers = req.headers().clone();

    // Build reqwest request
    let mut builder = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
        target_url,
    );

    // Copy headers (except Host — set to original for SNI routing at agent)
    for (name, value) in &headers {
        if name == "host" {
            continue;
        }
        if let Ok(v) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
            builder = builder.header(name.as_str(), v);
        }
    }
    // Set Host header to the original domain so the agent can route by SNI/Host
    builder = builder.header("Host", original_host);

    // Stream the body
    let body_stream = req.into_body();
    let body_bytes = axum::body::to_bytes(body_stream, 100 * 1024 * 1024)
        .await
        .map_err(|e| format!("Failed to read request body: {}", e))?;
    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes);
    }

    let resp = client
        .execute(builder.build().map_err(|e| e.to_string())?)
        .await
        .map_err(|e| e.to_string())?;

    // Convert reqwest Response → axum Response
    let status = StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let mut response_builder = Response::builder().status(status);

    for (name, value) in resp.headers() {
        if let Ok(hv) = HeaderValue::from_bytes(value.as_bytes()) {
            response_builder = response_builder.header(name.as_str(), hv);
        }
    }

    let body_bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    response_builder
        .body(Body::from(body_bytes))
        .map_err(|e| e.to_string())
}

/// Handle WebSocket upgrade to a TLS backend (re-encrypt).
/// Uses tokio-rustls to establish a TLS connection to the backend,
/// then performs the HTTP/1.1 upgrade handshake and bridges the streams.
async fn handle_websocket_upgrade_tls(
    mut req: Request,
    route: &RouteConfig,
    target_uri: Uri,
    original_host: &str,
) -> Result<Response, ProxyError> {
    use hyper::client::conn::http1::Builder;
    use tokio::io::AsyncWriteExt;
    use tokio_rustls::TlsConnector;

    let client_upgrade = hyper::upgrade::on(&mut req);

    let backend_addr = format!("{}:{}", route.target_host, route.target_port);
    let tcp_stream = TcpStream::connect(&backend_addr)
        .await
        .map_err(|e| {
            ProxyError::UpstreamError(format!(
                "Failed to connect to backend {}: {}",
                backend_addr, e
            ))
        })?;

    // Build a TLS config that skips certificate verification (trusted LAN)
    let tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(tls_config));

    let server_name = rustls::pki_types::ServerName::try_from(original_host.to_string())
        .unwrap_or_else(|_| rustls::pki_types::ServerName::try_from("localhost".to_string()).unwrap());

    let tls_stream = connector
        .connect(server_name, tcp_stream)
        .await
        .map_err(|e| {
            ProxyError::UpstreamError(format!(
                "TLS handshake to backend {} failed: {}",
                backend_addr, e
            ))
        })?;

    let io = TokioIo::new(tls_stream);

    let (mut sender, conn) = Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .handshake(io)
        .await
        .map_err(|e| ProxyError::UpstreamError(format!("Backend handshake failed: {}", e)))?;

    tokio::spawn(async move {
        if let Err(e) = conn.with_upgrades().await {
            let msg = e.to_string();
            if !msg.contains("connection closed") && !msg.contains("not connected") {
                error!("WebSocket TLS backend connection error: {}", e);
            }
        }
    });

    *req.uri_mut() = target_uri;

    let backend_response = sender
        .send_request(req)
        .await
        .map_err(|e| ProxyError::UpstreamError(format!("Backend request failed: {}", e)))?;

    if backend_response.status() != StatusCode::SWITCHING_PROTOCOLS {
        warn!(
            "TLS backend did not upgrade WebSocket, status: {}",
            backend_response.status()
        );
        return Ok(backend_response.into_response());
    }

    info!("WebSocket TLS upgrade successful to {}", backend_addr);

    let mut response_builder = Response::builder().status(StatusCode::SWITCHING_PROTOCOLS);

    for (name, value) in backend_response.headers() {
        response_builder = response_builder.header(name, value);
    }

    let backend_upgrade = hyper::upgrade::on(backend_response);

    let client_response = response_builder.body(Body::empty()).unwrap();

    tokio::spawn(async move {
        match tokio::try_join!(client_upgrade, backend_upgrade) {
            Ok((client_io, backend_io)) => {
                let mut client_io = TokioIo::new(client_io);
                let mut backend_io = TokioIo::new(backend_io);
                match tokio::io::copy_bidirectional(&mut client_io, &mut backend_io).await {
                    Ok((from_client, from_backend)) => {
                        debug!(
                            "WebSocket TLS closed: {} bytes client->backend, {} bytes backend->client",
                            from_client, from_backend
                        );
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if !msg.contains("connection reset") && !msg.contains("broken pipe") {
                            debug!("WebSocket TLS IO error: {}", e);
                        }
                    }
                }
                let _ = client_io.shutdown().await;
                let _ = backend_io.shutdown().await;
            }
            Err(e) => {
                error!("WebSocket TLS upgrade bridging failed: {}", e);
            }
        }
    });

    Ok(client_response)
}

/// Certificate verifier that accepts any certificate (for trusted LAN backends).
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
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
    AuthRequired(Option<String>),

    #[error("Forbidden")]
    Forbidden,

    #[error("Domain not found: {0}")]
    DomainNotFound(String),
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        match self {
            ProxyError::AuthRequired(Some(redirect_url)) => Response::builder()
                .status(StatusCode::FOUND)
                .header("Location", &redirect_url)
                .body(Body::empty())
                .unwrap(),
            other => {
                let (status, message) = match other {
                    ProxyError::InvalidUri(msg) => (StatusCode::BAD_REQUEST, msg),
                    ProxyError::UpstreamError(msg) => (StatusCode::BAD_GATEWAY, msg),
                    ProxyError::Forbidden => {
                        (StatusCode::FORBIDDEN, "Forbidden".to_string())
                    }
                    ProxyError::DomainNotFound(domain) => (
                        StatusCode::NOT_FOUND,
                        format!("Domain not configured: {}", domain),
                    ),
                    _ => (
                        StatusCode::UNAUTHORIZED,
                        "Authentication required".to_string(),
                    ),
                };
                (status, message).into_response()
            }
        }
    }
}

/// Check if an error message indicates connection failure (for Wake-on-Demand).
/// Triggers on connection refused, connection reset, or generic connect errors
/// which typically mean the backend service is down.
fn is_connection_refused(err: &str) -> bool {
    err.contains("Connection refused")
        || err.contains("connection refused")
        || err.contains("os error 111") // ECONNREFUSED on Linux
        || err.contains("client error (Connect)")  // hyper-util connect error
        || err.contains("Failed to connect to backend") // WebSocket connect error
}

/// SSE endpoint for Wake-on-Demand: streams host power + service start events.
/// When the host transitions to Online (after WOL), sends ServiceCommand::Start
/// for the relevant service type. Uses TCP polling to detect backend readiness.
async fn handle_wod_sse(
    state: &Arc<ProxyState>,
    app_route: &AppRoute,
) -> Result<Response, ProxyError> {
    use futures_util::StreamExt;

    let (tx, rx) = tokio::sync::mpsc::channel::<String>(16);

    let target_ip = app_route.target_ip;
    let target_port = app_route.target_port;
    let events = state.get_events();
    let registry = state.get_registry();
    let app_id = app_route.app_id.clone();
    let host_id = app_route.host_id.clone();
    let svc_type_enum = app_route.service_type;
    let svc_type = format!("{:?}", app_route.service_type).to_lowercase();

    tokio::spawn(async move {
        let timeout = tokio::time::sleep(std::time::Duration::from_secs(180));
        tokio::pin!(timeout);

        let mut event_sub = events.as_ref().map(|e| e.service_command.subscribe());
        let mut power_sub = events.as_ref().map(|e| e.host_power.subscribe());
        let mut poll_interval = tokio::time::interval(std::time::Duration::from_millis(1500));
        poll_interval.tick().await; // consume first immediate tick
        let mut service_start_sent = false;

        loop {
            tokio::select! {
                _ = &mut timeout => {
                    let _ = tx.send("data: {\"type\":\"error\",\"message\":\"timeout\"}\n\n".to_string()).await;
                    break;
                }
                // Poll the actual backend port for readiness
                _ = poll_interval.tick() => {
                    let addr = format!("{}:{}", target_ip, target_port);
                    if tokio::net::TcpStream::connect(&addr).await.is_ok() {
                        let _ = tx.send("data: {\"type\":\"ready\"}\n\n".to_string()).await;
                        break;
                    }
                }
                // Host power state changes (WOL → Online → etc.)
                result = async {
                    if let Some(ref mut sub) = power_sub {
                        sub.recv().await
                    } else {
                        std::future::pending().await
                    }
                } => {
                    if let Ok(event) = result {
                        if event.host_id == host_id {
                            let msg = format!(
                                "data: {{\"type\":\"power\",\"state\":\"{}\",\"message\":\"{}\"}}\n\n",
                                event.state, event.message
                            );
                            let _ = tx.send(msg).await;

                            // When host comes online after WOL, send ServiceCommand::Start
                            if event.state == HostPowerState::Online && !service_start_sent {
                                service_start_sent = true;
                                if let Some(ref reg) = registry {
                                    let reg = reg.clone();
                                    let aid = app_id.clone();
                                    tokio::spawn(async move {
                                        let _ = reg.send_service_command(&aid, svc_type_enum, ServiceAction::Start).await;
                                    });
                                }
                            }
                        }
                    }
                }
                // Service command events for UI feedback
                result = async {
                    if let Some(ref mut sub) = event_sub {
                        sub.recv().await
                    } else {
                        std::future::pending().await
                    }
                } => {
                    if let Ok(event) = result {
                        if event.app_id == app_id && event.service_type == svc_type && event.action != "started" {
                            let msg = format!(
                                "data: {{\"type\":\"waking\",\"service\":\"{}\",\"state\":\"{}\"}}\n\n",
                                event.service_type, event.action
                            );
                            let _ = tx.send(msg).await;
                        }
                    }
                }
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let body = Body::from_stream(stream.map(|s| Ok::<_, std::io::Error>(s)));

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(body)
        .unwrap())
}

/// Handle Wake-on-Demand for an app route using the host power state machine.
/// Applies to all service types (App, CodeServer, Db).
/// Dispatches to WoL for offline/suspended remote hosts, rejects during
/// shutdown/suspend transitions, and sends ServiceCommand::Start for online hosts.
async fn handle_wod(
    state: &Arc<ProxyState>,
    app_route: &AppRoute,
    host: &str,
) -> Response {
    if app_route.host_id != "local" {
        if let Some(registry) = state.get_registry() {
            let host_id = app_route.host_id.clone();
            let power_state = registry.get_host_power_state(&host_id).await;

            match power_state {
                HostPowerState::Offline | HostPowerState::Suspended => {
                    // Try to send WOL (handles dedup internally)
                    let wake_msg = match registry.request_wake_host(&host_id).await {
                        Ok(_) => "Reveil de l'hote en cours...",
                        Err(e) => {
                            warn!("WOL request failed for {}: {}", host_id, e);
                            "Reveil de l'hote en cours..."
                        }
                    };
                    if app_route.wake_page_enabled {
                        return wake_on_demand_page(host, wake_msg);
                    } else {
                        return handle_wod_transparent(state, app_route).await;
                    }
                }
                HostPowerState::WakingUp => {
                    // Already waking — show wake page without sending another WOL
                    if app_route.wake_page_enabled {
                        return wake_on_demand_page(host, "Reveil de l'hote en cours...");
                    } else {
                        return handle_wod_transparent(state, app_route).await;
                    }
                }
                HostPowerState::Rebooting => {
                    if app_route.wake_page_enabled {
                        return wake_on_demand_page(host, "Redemarrage de l'hote en cours...");
                    } else {
                        return handle_wod_transparent(state, app_route).await;
                    }
                }
                HostPowerState::ShuttingDown | HostPowerState::Suspending => {
                    // Active power action — return 503 immediately
                    return Response::builder()
                        .status(StatusCode::SERVICE_UNAVAILABLE)
                        .header("Retry-After", "10")
                        .header("Content-Type", "text/plain")
                        .body(Body::from("Host power action in progress"))
                        .unwrap();
                }
                HostPowerState::Online => {
                    // Host is online but service is down — start the service
                    let app_id = app_route.app_id.clone();
                    let svc = app_route.service_type;
                    tokio::spawn(async move {
                        let _ = registry.send_service_command(&app_id, svc, ServiceAction::Start).await;
                    });
                    if app_route.wake_page_enabled {
                        return wake_on_demand_page(host, "Demarrage du service...");
                    } else {
                        return handle_wod_transparent(state, app_route).await;
                    }
                }
            }
        }
    } else {
        // Local host — start service
        if let Some(registry) = state.get_registry() {
            let app_id = app_route.app_id.clone();
            let svc = app_route.service_type;
            tokio::spawn(async move {
                let _ = registry.send_service_command(&app_id, svc, ServiceAction::Start).await;
            });
        }
    }
    if app_route.wake_page_enabled {
        wake_on_demand_page(host, "Demarrage du service...")
    } else {
        handle_wod_transparent(state, app_route).await
    }
}

/// Transparent Wake-on-Demand: holds the connection, polls until the backend port
/// is actually listening, then returns a Retry-After:0 response so the browser
/// retries immediately. Timeout extended to 180s for WOL boot sequences.
async fn handle_wod_transparent(
    _state: &Arc<ProxyState>,
    app_route: &AppRoute,
) -> Response {
    let target_ip = app_route.target_ip;
    let target_port = app_route.target_port;
    let addr = format!("{}:{}", target_ip, target_port);

    let timeout = tokio::time::sleep(std::time::Duration::from_secs(180));
    tokio::pin!(timeout);
    let mut poll_interval = tokio::time::interval(std::time::Duration::from_millis(1500));
    poll_interval.tick().await; // consume first immediate tick

    loop {
        tokio::select! {
            _ = &mut timeout => break,
            _ = poll_interval.tick() => {
                if tokio::net::TcpStream::connect(&addr).await.is_ok() {
                    return Response::builder()
                        .status(StatusCode::SERVICE_UNAVAILABLE)
                        .header("Retry-After", "0")
                        .header("Content-Type", "text/plain")
                        .body(Body::from("Service starting, please retry"))
                        .unwrap();
                }
            }
        }
    }

    // Timeout — return 503
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header("Retry-After", "5")
        .header("Content-Type", "text/plain")
        .body(Body::from("Service unavailable"))
        .unwrap()
}

/// Serve a Wake-on-Demand page that uses SSE to know when the service is ready.
/// Handles both power state events (host boot progress) and service state events.
fn wake_on_demand_page(host: &str, message: &str) -> Response {
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="fr">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Demarrage en cours...</title>
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;
background:#0f172a;color:#e2e8f0;display:flex;justify-content:center;align-items:center;
min-height:100vh}}
.card{{background:#1e293b;border-radius:16px;padding:3rem;text-align:center;
max-width:420px;box-shadow:0 25px 50px rgba(0,0,0,.3)}}
.spinner{{width:48px;height:48px;border:4px solid #334155;border-top-color:#3b82f6;
border-radius:50%;animation:spin 1s linear infinite;margin:0 auto 1.5rem}}
@keyframes spin{{to{{transform:rotate(360deg)}}}}
h1{{font-size:1.25rem;font-weight:600;margin-bottom:.75rem}}
p#status{{color:#94a3b8;font-size:.9rem;line-height:1.5}}
.host{{color:#60a5fa;font-family:monospace;font-size:.85rem;margin-top:1rem}}
</style>
</head>
<body>
<div class="card">
<div class="spinner"></div>
<h1 id="title">{message}</h1>
<p id="status">Connexion au service...</p>
<div class="host">{host}</div>
</div>
<script>
var es = new EventSource('/__hr/wod');
es.onmessage = function(e) {{
  var msg = JSON.parse(e.data);
  if (msg.type === 'power') {{
    if (msg.state === 'online') {{
      document.getElementById('title').textContent = 'Hote en ligne';
      document.getElementById('status').textContent = 'Demarrage des services...';
    }} else if (msg.state === 'waking_up') {{
      document.getElementById('status').textContent = msg.message || 'En attente du demarrage...';
    }} else if (msg.state === 'offline') {{
      document.getElementById('status').textContent = msg.message || 'Hote hors ligne';
    }}
  }} else if (msg.type === 'waking') {{
    document.getElementById('status').textContent = 'Demarrage ' + msg.service + '...';
  }} else if (msg.type === 'ready') {{
    es.close();
    location.reload();
  }} else if (msg.type === 'error') {{
    document.getElementById('status').textContent = msg.message;
    es.close();
  }}
}};
es.onerror = function() {{ es.close(); setTimeout(function(){{ location.reload(); }}, 5000); }};
</script>
</body>
</html>"#,
        host = host,
        message = message,
    );

    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header("Content-Type", "text/html; charset=utf-8")
        .header("Retry-After", "3")
        .body(Body::from(html))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ProxyConfig, RouteConfig};
    use std::path::PathBuf;

    fn test_config() -> ProxyConfig {
        ProxyConfig {
            http_port: 80,
            https_port: 443,
            base_domain: "example.com".to_string(),
            tls_mode: "local-ca".to_string(),
            ca_storage_path: PathBuf::from("/tmp/ca"),
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
            ],
            access_log_path: None,
        }
    }

    #[test]
    fn test_find_route_by_domain() {
        let state = ProxyState::new(test_config(), 4000);
        let route = state.find_route("app.example.com");
        assert!(route.is_some());
        assert_eq!(route.unwrap().target_port, 3000);
    }

    #[test]
    fn test_find_route_strips_port() {
        let state = ProxyState::new(test_config(), 4000);
        let route = state.find_route("app.example.com:444");
        assert!(route.is_some());
        assert_eq!(route.unwrap().domain, "app.example.com");
    }

    #[test]
    fn test_find_route_unknown_domain() {
        let state = ProxyState::new(test_config(), 4000);
        assert!(state.find_route("unknown.example.com").is_none());
    }

    #[test]
    fn test_find_route_disabled() {
        let state = ProxyState::new(test_config(), 4000);
        assert!(state.find_route("disabled.example.com").is_none());
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
    fn test_not_websocket() {
        let req = Request::builder()
            .header("connection", "keep-alive")
            .body(Body::empty())
            .unwrap();
        assert!(!is_websocket_upgrade(&req));

        let req = Request::builder()
            .header("upgrade", "h2c")
            .header("connection", "Upgrade")
            .body(Body::empty())
            .unwrap();
        assert!(!is_websocket_upgrade(&req));
    }

    #[test]
    fn test_proxy_error_status_codes() {
        let err = ProxyError::DomainNotFound("test.com".to_string());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let err = ProxyError::Forbidden;
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        let err = ProxyError::AuthRequired(Some("https://auth.example.com/login".to_string()));
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::FOUND);

        let err = ProxyError::UpstreamError("timeout".to_string());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn test_reload_config() {
        let mut config = test_config();
        let state = ProxyState::new(config.clone(), 4000);
        assert!(state.find_route("app.example.com").is_some());
        assert!(state.find_route("new.example.com").is_none());

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
        assert_eq!(
            state.find_route("new.example.com").unwrap().target_port,
            5000
        );
    }
}
