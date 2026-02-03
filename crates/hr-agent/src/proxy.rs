//! HTTPS reverse proxy with SNI-based multi-domain routing.
//! Each domain gets its own certificate and routes to a specific localhost port.
//! Includes powersave integration: wake-on-request and activity tracking.

use std::collections::HashMap;
use std::io::BufReader;
use std::net::{Ipv6Addr, SocketAddr};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::ServerConfig;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, error, info, warn};

use hr_registry::protocol::{AgentRoute, ServiceType};

use crate::pages;
use crate::powersave::{PowersaveManager, WakeResult};

/// Interval for WebSocket keep-alive activity recording.
const WS_KEEPALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Port that code-server listens on.
const CODE_SERVER_PORT: u16 = 13337;

/// Route configuration for a single domain
#[derive(Clone)]
struct RouteEntry {
    target_port: u16,
    auth_required: bool,
    allowed_groups: Vec<String>,
    /// Service type for powersave (determined by port).
    service_type: ServiceType,
}

/// SNI resolver for the agent's multi-domain TLS
#[derive(Debug)]
struct AgentSniResolver {
    certs: RwLock<HashMap<String, Arc<CertifiedKey>>>,
}

impl AgentSniResolver {
    fn new() -> Self {
        Self {
            certs: RwLock::new(HashMap::new()),
        }
    }

    fn insert(&self, domain: String, key: Arc<CertifiedKey>) {
        self.certs.write().unwrap().insert(domain, key);
    }

    fn clear(&self) {
        self.certs.write().unwrap().clear();
    }
}

impl ResolvesServerCert for AgentSniResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let server_name = client_hello.server_name()?;
        let certs = self.certs.read().unwrap();
        let key = certs.get(server_name).cloned();
        if key.is_none() {
            warn!("No certificate for SNI: {server_name}");
        }
        key
    }
}

/// Shared state for the proxy
pub struct ProxyState {
    routes: RwLock<HashMap<String, RouteEntry>>,
    auth_url: RwLock<String>,
    /// Optional powersave manager for wake-on-request.
    powersave: RwLock<Option<Arc<PowersaveManager>>>,
    /// Dashboard URL for down page links.
    dashboard_url: RwLock<String>,
    /// App ID (service_name/slug) for filtering WebSocket metrics.
    app_id: RwLock<String>,
}

impl ProxyState {
    fn new() -> Self {
        Self {
            routes: RwLock::new(HashMap::new()),
            auth_url: RwLock::new(String::new()),
            powersave: RwLock::new(None),
            dashboard_url: RwLock::new(String::new()),
            app_id: RwLock::new(String::new()),
        }
    }

    fn find_route(&self, host: &str) -> Option<RouteEntry> {
        let domain = host.split(':').next().unwrap_or(host);
        self.routes.read().unwrap().get(domain).cloned()
    }

    fn auth_url(&self) -> String {
        self.auth_url.read().unwrap().clone()
    }

    fn powersave(&self) -> Option<Arc<PowersaveManager>> {
        self.powersave.read().unwrap().clone()
    }

    fn dashboard_url(&self) -> String {
        self.dashboard_url.read().unwrap().clone()
    }

    fn app_id(&self) -> String {
        self.app_id.read().unwrap().clone()
    }

    /// Set the powersave manager.
    pub fn set_powersave(&self, pm: Arc<PowersaveManager>) {
        *self.powersave.write().unwrap() = Some(pm);
    }

    /// Set the dashboard URL.
    pub fn set_dashboard_url(&self, url: String) {
        *self.dashboard_url.write().unwrap() = url;
    }

    /// Set the app ID (service_name/slug).
    pub fn set_app_id(&self, id: String) {
        *self.app_id.write().unwrap() = id;
    }
}

/// The running proxy handle
pub struct AgentProxy {
    state: Arc<ProxyState>,
    resolver: Arc<AgentSniResolver>,
    tls_config: Arc<ServerConfig>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
}

impl AgentProxy {
    /// Create a new proxy (not yet listening).
    pub fn new() -> Result<Self> {
        let resolver = Arc::new(AgentSniResolver::new());
        let state = Arc::new(ProxyState::new());

        let tls_config = Arc::new(
            ServerConfig::builder()
                .with_no_client_auth()
                .with_cert_resolver(resolver.clone()),
        );

        Ok(Self {
            state,
            resolver,
            tls_config,
            shutdown_tx: None,
        })
    }

    /// Update routes and certificates from a Config message.
    pub fn apply_routes(&self, routes: &[AgentRoute], auth_url: &str) -> Result<()> {
        // Clear and reload SNI resolver
        self.resolver.clear();
        let mut route_map = HashMap::new();

        for route in routes {
            // Parse PEM cert + key
            let cert_key = load_certified_key(&route.cert_pem, &route.key_pem)
                .with_context(|| format!("Failed to load cert for {}", route.domain))?;

            self.resolver
                .insert(route.domain.clone(), Arc::new(cert_key));

            // Determine service type based on port
            let service_type = if route.target_port == CODE_SERVER_PORT {
                ServiceType::CodeServer
            } else {
                ServiceType::App
            };

            route_map.insert(
                route.domain.clone(),
                RouteEntry {
                    target_port: route.target_port,
                    auth_required: route.auth_required,
                    allowed_groups: route.allowed_groups.clone(),
                    service_type,
                },
            );

            info!(domain = route.domain, port = route.target_port, service_type = ?service_type, "Route configured");
        }

        *self.state.routes.write().unwrap() = route_map;
        *self.state.auth_url.write().unwrap() = auth_url.to_string();

        info!(count = routes.len(), "Proxy routes updated");
        Ok(())
    }

    /// Get a reference to the proxy state for external configuration.
    pub fn state(&self) -> &Arc<ProxyState> {
        &self.state
    }

    /// Spawn the proxy listener in a background task. Returns the JoinHandle.
    pub fn spawn_listener(&mut self, bind_addr: Ipv6Addr) -> Result<tokio::task::JoinHandle<()>> {
        let addr = SocketAddr::from((bind_addr, 443));
        let tls_config = self.tls_config.clone();
        let state = self.state.clone();

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        let handle = tokio::spawn(async move {
            // Retry bind up to 5 times (address may not be ready yet after ip addr add)
            let listener = {
                let mut last_err = None;
                let mut bound = None;
                for attempt in 0..5 {
                    match TcpListener::bind(addr).await {
                        Ok(l) => {
                            bound = Some(l);
                            break;
                        }
                        Err(e) => {
                            warn!(addr = %addr, attempt, "Bind failed, retrying: {e}");
                            last_err = Some(e);
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        }
                    }
                }
                match bound {
                    Some(l) => l,
                    None => {
                        error!(addr = %addr, "Failed to bind proxy after retries: {}", last_err.unwrap());
                        return;
                    }
                }
            };

            let acceptor = TlsAcceptor::from(tls_config);
            info!(addr = %addr, "Agent HTTPS proxy listening");

            let mut shutdown_rx = shutdown_rx;
            loop {
                tokio::select! {
                    accept_result = listener.accept() => {
                        let (tcp_stream, remote_addr) = match accept_result {
                            Ok(r) => r,
                            Err(e) => {
                                warn!("TCP accept error: {e}");
                                continue;
                            }
                        };

                        let acceptor = acceptor.clone();
                        let state = state.clone();

                        tokio::spawn(async move {
                            let tls_stream = match acceptor.accept(tcp_stream).await {
                                Ok(s) => s,
                                Err(e) => {
                                    debug!("TLS handshake failed from {remote_addr}: {e}");
                                    return;
                                }
                            };

                            let io = TokioIo::new(tls_stream);
                            let client_ip = remote_addr.ip();

                            let service = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                                let state = state.clone();
                                async move {
                                    let resp = handle_request(state, client_ip, req).await;
                                    Ok::<_, std::convert::Infallible>(resp)
                                }
                            });

                            if let Err(e) = http1::Builder::new()
                                .preserve_header_case(true)
                                .title_case_headers(true)
                                .serve_connection(io, service)
                                .with_upgrades()
                                .await
                            {
                                let msg = e.to_string();
                                if !msg.contains("connection closed")
                                    && !msg.contains("not connected")
                                    && !msg.contains("connection reset")
                                {
                                    debug!("HTTP/1 error from {remote_addr}: {e}");
                                }
                            }
                        });
                    }

                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            info!("Proxy shutdown signal received");
                            break;
                        }
                    }
                }
            }
        });

        Ok(handle)
    }

    /// Signal the proxy to stop.
    pub fn shutdown(&self) {
        if let Some(tx) = &self.shutdown_tx {
            let _ = tx.send(true);
        }
    }
}

/// Handle a single HTTP request after TLS termination.
async fn handle_request(
    state: Arc<ProxyState>,
    client_ip: std::net::IpAddr,
    mut req: hyper::Request<hyper::body::Incoming>,
) -> hyper::Response<http_body_util::combinators::BoxBody<hyper::body::Bytes, std::convert::Infallible>> {
    use http_body_util::{BodyExt, Empty, Full};

    // Internal status endpoint for loading page polling
    if req.uri().path() == "/_hr/status" {
        return handle_status_request(&state).await;
    }

    let host = req
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let route = match state.find_route(&host) {
        Some(r) => r,
        None => {
            warn!(host, "No route found");
            return hyper::Response::builder()
                .status(404)
                .body(Full::new(hyper::body::Bytes::from("Domain not configured"))
                    .map_err(|never: std::convert::Infallible| match never {})
                    .boxed())
                .unwrap();
        }
    };

    // Check powersave state and wake if needed
    if let Some(powersave) = state.powersave() {
        // Only show loading/down pages for document requests (HTML navigation)
        // For assets (JS/CSS/images), just proxy through and let them fail normally
        let is_document_request = req
            .headers()
            .get("accept")
            .and_then(|v| v.to_str().ok())
            .map(|accept| accept.contains("text/html"))
            .unwrap_or(false);

        let service_name = match route.service_type {
            ServiceType::CodeServer => "Code Server (IDE)",
            ServiceType::App => "Application",
            ServiceType::Db => "Database",
        };

        let dashboard = state.dashboard_url();
        let app_id = state.app_id();
        match powersave.ensure_running(route.service_type, route.target_port).await {
            WakeResult::AlreadyRunning => {
                // Continue to proxy
            }
            WakeResult::Starting => {
                if is_document_request {
                    // Return loading page with WebSocket connection to dashboard
                    return pages::loading_response(service_name, &dashboard, &app_id);
                }
                // For non-document requests, continue to proxy (will fail with 502 if backend not ready)
            }
            WakeResult::ManuallyOff => {
                if is_document_request {
                    // Return down page
                    return pages::down_response(service_name, &dashboard);
                }
                // For non-document requests, return 503
                return hyper::Response::builder()
                    .status(503)
                    .body(Full::new(hyper::body::Bytes::from("Service unavailable"))
                        .map_err(|never: std::convert::Infallible| match never {})
                        .boxed())
                    .unwrap();
            }
        }
    }

    // Forward-auth check
    if route.auth_required {
        let auth_url = state.auth_url();
        if !auth_url.is_empty() {
            match forward_auth_check(&auth_url, &host, &req).await {
                AuthCheckResult::Ok => {}
                AuthCheckResult::Redirect(url) => {
                    return hyper::Response::builder()
                        .status(302)
                        .header("Location", &url)
                        .body(Empty::new()
                            .map_err(|never| match never {})
                            .boxed())
                        .unwrap();
                }
                AuthCheckResult::Forbidden => {
                    return hyper::Response::builder()
                        .status(403)
                        .body(Full::new(hyper::body::Bytes::from("Forbidden"))
                            .map_err(|never| match never {})
                            .boxed())
                        .unwrap();
                }
                AuthCheckResult::Error(e) => {
                    error!("Forward-auth error: {e}");
                    return hyper::Response::builder()
                        .status(502)
                        .body(Full::new(hyper::body::Bytes::from("Auth service error"))
                            .map_err(|never| match never {})
                            .boxed())
                        .unwrap();
                }
            }
        }
    }

    // Check for WebSocket upgrade before mutating headers
    let is_websocket = is_websocket_upgrade(&req);

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|x| x.to_string())
        .unwrap_or_else(|| "/".to_string());

    // Set forwarding headers
    {
        let headers = req.headers_mut();
        if let Ok(val) = hyper::header::HeaderValue::from_str(&host) {
            headers.insert("X-Forwarded-Host", val);
        }
        headers.insert("X-Forwarded-Proto", hyper::header::HeaderValue::from_static("https"));
        if let Ok(val) = hyper::header::HeaderValue::from_str(&client_ip.to_string()) {
            headers.insert("X-Forwarded-For", val.clone());
            headers.insert("X-Real-IP", val);
        }
    }

    let target_url = format!("http://127.0.0.1:{}{}", route.target_port, &path_and_query);

    if is_websocket {
        return handle_websocket_upgrade(
            req,
            route.target_port,
            &path_and_query,
            state.powersave(),
            route.service_type,
        )
        .await;
    }

    // Normal HTTP proxy
    let target_uri: hyper::Uri = match target_url.parse() {
        Ok(u) => u,
        Err(e) => {
            return hyper::Response::builder()
                .status(400)
                .body(Full::new(hyper::body::Bytes::from(format!("Invalid URI: {e}")))
                    .map_err(|never: std::convert::Infallible| match never {})
                    .boxed())
                .unwrap();
        }
    };

    {
        let headers = req.headers_mut();
        headers.remove("connection");
        headers.remove("upgrade");
    }
    *req.uri_mut() = target_uri;

    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build_http();

    match client.request(req).await {
        Ok(resp) => {
            // Add anti-cache headers for HTML responses to ensure fresh content
            // when services restart after being stopped
            let is_html = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(|ct| ct.contains("text/html"))
                .unwrap_or(false);

            if is_html {
                let (mut parts, body) = resp.into_parts();
                parts.headers.insert(
                    "Cache-Control",
                    hyper::header::HeaderValue::from_static("no-cache, no-store, must-revalidate"),
                );
                parts
                    .headers
                    .insert("Pragma", hyper::header::HeaderValue::from_static("no-cache"));
                parts
                    .headers
                    .insert("Expires", hyper::header::HeaderValue::from_static("0"));
                hyper::Response::from_parts(parts, body.map_err(|_| unreachable!()).boxed())
            } else {
                resp.map(|b| b.map_err(|_| unreachable!()).boxed())
            }
        }
        Err(e) => {
            error!(target_url, "Upstream error: {e}");
            hyper::Response::builder()
                .status(502)
                .body(Full::new(hyper::body::Bytes::from(format!("Upstream error: {e}")))
                    .map_err(|never: std::convert::Infallible| match never {})
                    .boxed())
                .unwrap()
        }
    }
}

/// Handle WebSocket upgrade by establishing a direct connection to the backend
async fn handle_websocket_upgrade(
    mut req: hyper::Request<hyper::body::Incoming>,
    target_port: u16,
    path_and_query: &str,
    powersave: Option<Arc<PowersaveManager>>,
    service_type: ServiceType,
) -> hyper::Response<http_body_util::combinators::BoxBody<hyper::body::Bytes, std::convert::Infallible>> {
    use http_body_util::{BodyExt, Empty, Full};
    use hyper::client::conn::http1::Builder;
    use tokio::io::AsyncWriteExt;

    let client_upgrade = hyper::upgrade::on(&mut req);

    let backend_addr = format!("127.0.0.1:{target_port}");
    let tcp_stream = match tokio::net::TcpStream::connect(&backend_addr).await {
        Ok(s) => s,
        Err(e) => {
            return hyper::Response::builder()
                .status(502)
                .body(Full::new(hyper::body::Bytes::from(format!("Backend connect failed: {e}")))
                    .map_err(|never: std::convert::Infallible| match never {})
                    .boxed())
                .unwrap();
        }
    };

    let io = TokioIo::new(tcp_stream);
    let (mut sender, conn) = match Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .handshake(io)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return hyper::Response::builder()
                .status(502)
                .body(Full::new(hyper::body::Bytes::from(format!("Backend handshake failed: {e}")))
                    .map_err(|never: std::convert::Infallible| match never {})
                    .boxed())
                .unwrap();
        }
    };

    tokio::spawn(async move {
        if let Err(e) = conn.with_upgrades().await {
            let msg = e.to_string();
            if !msg.contains("connection closed") && !msg.contains("not connected") {
                error!("WebSocket backend connection error: {e}");
            }
        }
    });

    let target_uri: hyper::Uri = path_and_query
        .parse()
        .unwrap_or_else(|_| "/".parse().unwrap());
    *req.uri_mut() = target_uri;

    // Set Host header to backend address for code-server compatibility
    if let Ok(val) = hyper::header::HeaderValue::from_str(&backend_addr) {
        req.headers_mut().insert("host", val);
    }

    let backend_response = match sender.send_request(req).await {
        Ok(r) => r,
        Err(e) => {
            return hyper::Response::builder()
                .status(502)
                .body(Full::new(hyper::body::Bytes::from(format!("Backend request failed: {e}")))
                    .map_err(|never: std::convert::Infallible| match never {})
                    .boxed())
                .unwrap();
        }
    };

    if backend_response.status() != hyper::StatusCode::SWITCHING_PROTOCOLS {
        return backend_response.map(|b| b.map_err(|_| unreachable!()).boxed());
    }

    info!("WebSocket upgrade successful to {backend_addr}");

    let mut response_builder =
        hyper::Response::builder().status(hyper::StatusCode::SWITCHING_PROTOCOLS);
    for (name, value) in backend_response.headers() {
        response_builder = response_builder.header(name, value);
    }

    let backend_upgrade = hyper::upgrade::on(backend_response);

    let client_response = response_builder
        .body(Empty::new().map_err(|never| match never {}).boxed())
        .unwrap();

    // Notification to signal when the WebSocket connection is closed
    let shutdown = Arc::new(tokio::sync::Notify::new());

    // Spawn keep-alive task that records activity periodically
    if let Some(pm) = powersave {
        let pm_clone = pm.clone();
        let shutdown_clone = shutdown.clone();
        debug!(service_type = ?service_type, "WebSocket keep-alive task started");
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = shutdown_clone.notified() => {
                        break;
                    }
                    _ = tokio::time::sleep(WS_KEEPALIVE_INTERVAL) => {
                        debug!(service_type = ?service_type, "WebSocket keep-alive: recording activity");
                        pm_clone.record_activity(service_type);
                    }
                }
            }
            debug!(service_type = ?service_type, "WebSocket keep-alive task ended");
        });
    } else {
        warn!(service_type = ?service_type, "WebSocket: no powersave manager, keep-alive disabled");
    }

    tokio::spawn(async move {
        match tokio::try_join!(client_upgrade, backend_upgrade) {
            Ok((client_io, backend_io)) => {
                let mut client_io = TokioIo::new(client_io);
                let mut backend_io = TokioIo::new(backend_io);
                match tokio::io::copy_bidirectional(&mut client_io, &mut backend_io).await {
                    Ok((from_client, from_backend)) => {
                        debug!(
                            "WebSocket closed: {from_client}B client->backend, {from_backend}B backend->client"
                        );
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if !msg.contains("connection reset") && !msg.contains("broken pipe") {
                            debug!("WebSocket IO error: {e}");
                        }
                    }
                }
                let _ = client_io.shutdown().await;
                let _ = backend_io.shutdown().await;
            }
            Err(e) => {
                error!("WebSocket upgrade bridging failed: {e}");
            }
        }
        // Signal the keep-alive task to stop immediately
        shutdown.notify_one();
    });

    client_response
}

// ── Forward-auth ────────────────────────────────────────────

enum AuthCheckResult {
    Ok,
    Redirect(String),
    Forbidden,
    Error(String),
}

/// Call HomeRoute's forward-auth endpoint to check if the request is authenticated.
async fn forward_auth_check(
    auth_url: &str,
    host: &str,
    req: &hyper::Request<hyper::body::Incoming>,
) -> AuthCheckResult {
    let uri = req
        .uri()
        .path_and_query()
        .map(|pq| pq.to_string())
        .unwrap_or_else(|| "/".to_string());

    let cookie = req
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => return AuthCheckResult::Error(e.to_string()),
    };

    let resp = match client
        .get(auth_url)
        .header("X-Forwarded-Host", host)
        .header("X-Forwarded-Uri", &uri)
        .header("X-Forwarded-Proto", "https")
        .header("Cookie", cookie)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return AuthCheckResult::Error(e.to_string()),
    };

    match resp.status().as_u16() {
        200 => AuthCheckResult::Ok,
        401 => {
            // Extract login URL from response body or header
            if let Some(location) = resp.headers().get("Location") {
                if let Ok(url) = location.to_str() {
                    return AuthCheckResult::Redirect(url.to_string());
                }
            }
            // Try body
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(url) = body.get("login_url").and_then(|v| v.as_str()) {
                    return AuthCheckResult::Redirect(url.to_string());
                }
            }
            AuthCheckResult::Redirect(String::new())
        }
        403 => AuthCheckResult::Forbidden,
        status => AuthCheckResult::Error(format!("Unexpected auth status: {status}")),
    }
}

// ── Internal status endpoint ────────────────────────────────

/// Handle /_hr/status request - returns current service states as JSON.
async fn handle_status_request(
    state: &Arc<ProxyState>,
) -> hyper::Response<http_body_util::combinators::BoxBody<hyper::body::Bytes, std::convert::Infallible>> {
    use http_body_util::{BodyExt, Full};

    let (code_server_status, app_status, db_status) = if let Some(pm) = state.powersave() {
        (
            format!("{:?}", pm.get_state(ServiceType::CodeServer)).to_lowercase(),
            format!("{:?}", pm.get_state(ServiceType::App)).to_lowercase(),
            format!("{:?}", pm.get_state(ServiceType::Db)).to_lowercase(),
        )
    } else {
        ("unknown".to_string(), "unknown".to_string(), "unknown".to_string())
    };

    let json = format!(
        r#"{{"codeServerStatus":"{}","appStatus":"{}","dbStatus":"{}"}}"#,
        code_server_status, app_status, db_status
    );

    hyper::Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .header("Cache-Control", "no-cache, no-store, must-revalidate")
        .header("Access-Control-Allow-Origin", "*")
        .body(
            Full::new(hyper::body::Bytes::from(json))
                .map_err(|never: std::convert::Infallible| match never {})
                .boxed(),
        )
        .unwrap()
}

// ── Helpers ─────────────────────────────────────────────────

fn is_websocket_upgrade(req: &hyper::Request<hyper::body::Incoming>) -> bool {
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

/// Parse PEM-encoded cert chain + private key into a rustls CertifiedKey.
fn load_certified_key(cert_pem: &str, key_pem: &str) -> Result<CertifiedKey> {
    let certs: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut BufReader::new(cert_pem.as_bytes()))
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to parse certificate PEM")?;

    let key: PrivateKeyDer<'static> =
        rustls_pemfile::private_key(&mut BufReader::new(key_pem.as_bytes()))
            .context("Failed to parse private key PEM")?
            .ok_or_else(|| anyhow::anyhow!("No private key found in PEM"))?;

    let signing_key = rustls::crypto::ring::sign::any_supported_type(&key)
        .map_err(|e| anyhow::anyhow!("Failed to parse signing key: {e}"))?;

    Ok(CertifiedKey::new(certs, signing_key))
}
