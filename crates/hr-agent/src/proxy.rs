//! Mini HTTPS reverse proxy running inside the agent container.
//!
//! Terminates TLS on 0.0.0.0:443, routes by Host header to local services,
//! handles forward-auth via the central HomeRoute API, and supports WebSocket
//! upgrades.

use std::collections::HashMap;
use std::io::BufReader;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use anyhow::{Context, Result};
use hyper::body::Incoming;
use hyper::server::conn::http1::Builder as ServerBuilder;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use http_body_util::{BodyExt, Full};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, error, info, warn};

use crate::config::AgentConfig;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

fn full_body(body: impl Into<bytes::Bytes>) -> BoxBody {
    Full::new(body.into()).map_err(|never| match never {}).boxed()
}

fn empty_body() -> BoxBody {
    Full::new(bytes::Bytes::new()).map_err(|never| match never {}).boxed()
}

// ── Types ──────────────────────────────────────────────────────────

/// A local route: maps a Host header domain to a localhost port.
#[derive(Debug, Clone)]
struct LocalRoute {
    target_port: u16,
    auth_required: bool,
    allowed_groups: Vec<String>,
}

/// Cached forward-auth result.
#[derive(Debug, Clone)]
enum AuthResult {
    /// Authenticated user + groups.
    Ok { user: String, groups: String },
    /// Forbidden (wrong groups, etc.).
    Forbidden,
}

// ── SNI Resolver ───────────────────────────────────────────────────

/// Simplified SNI resolver holding at most 2 certs:
/// - app wildcard (*.{slug}.{base})
/// - global wildcard (*.{base})
#[derive(Debug)]
pub struct AgentSniResolver {
    certs: RwLock<HashMap<String, Arc<CertifiedKey>>>,
}

impl AgentSniResolver {
    fn new() -> Self {
        Self {
            certs: RwLock::new(HashMap::new()),
        }
    }

    /// Replace all certificates atomically.
    fn replace_all(&self, new_certs: HashMap<String, Arc<CertifiedKey>>) {
        let mut certs = self.certs.write().unwrap();
        *certs = new_certs;
    }
}

impl ResolvesServerCert for AgentSniResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let server_name = client_hello.server_name()?;
        let certs = self.certs.read().ok()?;

        // Try exact match first
        if let Some(key) = certs.get(server_name).cloned() {
            return Some(key);
        }

        // Walk up domain levels trying wildcard matches (most-specific first).
        // For "code.myapp.mynetwk.biz":
        //   1. Try *.myapp.mynetwk.biz  -> matches per-app cert
        //   2. Try *.mynetwk.biz        -> matches global cert
        let mut remaining = server_name;
        while let Some(dot_pos) = remaining.find('.') {
            let parent = &remaining[dot_pos + 1..];
            let wildcard = format!("*.{}", parent);
            if let Some(key) = certs.get(&wildcard).cloned() {
                return Some(key);
            }
            remaining = parent;
        }

        warn!("No certificate found for SNI: {}", server_name);
        None
    }
}

// ── AgentProxy ─────────────────────────────────────────────────────

/// The agent-side HTTPS reverse proxy.
pub struct AgentProxy {
    resolver: Arc<AgentSniResolver>,
    routes: Arc<RwLock<HashMap<String, LocalRoute>>>,
    auth_cache: Arc<RwLock<HashMap<String, (Instant, AuthResult)>>>,
    homeroute_url: String,
    agent_token: String,
}

impl AgentProxy {
    /// Create a new AgentProxy. Does NOT start the server yet.
    pub fn new(config: &AgentConfig) -> Self {
        let host = if config.homeroute_address.contains(':') {
            format!("[{}]", config.homeroute_address)
        } else {
            config.homeroute_address.clone()
        };
        Self {
            resolver: Arc::new(AgentSniResolver::new()),
            routes: Arc::new(RwLock::new(HashMap::new())),
            auth_cache: Arc::new(RwLock::new(HashMap::new())),
            homeroute_url: format!("http://{}:{}", host, config.homeroute_port),
            agent_token: config.token.clone(),
        }
    }

    /// Spawn the HTTPS server on 0.0.0.0:443. Returns the JoinHandle.
    pub fn start(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let proxy = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(e) = proxy.run_server().await {
                error!("Agent proxy server failed: {e}");
            }
        })
    }

    /// Update the route table from a Config message.
    pub fn update_routes(
        &self,
        base_domain: &str,
        slug: &str,
        frontend: Option<&hr_registry::types::FrontendEndpoint>,
        environment: hr_registry::types::Environment,
        code_server_enabled: bool,
    ) {
        let mut new_routes = HashMap::new();
        if let Some(fe) = frontend {
            // Only production gets a frontend route; dev containers have no public endpoint
            if environment == hr_registry::types::Environment::Production {
                let frontend_domain = format!("{}.{}", slug, base_domain);
                new_routes.insert(
                    frontend_domain,
                    LocalRoute {
                        target_port: fe.target_port,
                        auth_required: fe.auth_required,
                        allowed_groups: fe.allowed_groups.clone(),
                    },
                );
            }
        }
        if code_server_enabled && environment == hr_registry::types::Environment::Development {
            new_routes.insert(
                format!("code.{}.{}", slug, base_domain),
                LocalRoute {
                    target_port: 13337,
                    auth_required: true,
                    allowed_groups: vec![],
                },
            );
        }
        let count = new_routes.len();
        {
            let mut routes = self.routes.write().unwrap();
            *routes = new_routes;
        }
        info!("Agent proxy route table updated ({} routes)", count);
    }

    /// Pull certificates from the central server and load them into the SNI resolver.
    pub async fn update_certs(&self) -> Result<()> {
        let certs_json = pull_certs(&self.homeroute_url, &self.agent_token).await?;

        let mut new_certs = HashMap::new();

        // Parse app cert (*.{slug}.{base})
        // Server returns: { "app_cert": { "cert_pem": "...", "key_pem": "...", "wildcard_domain": "..." }, ... }
        if let Some(app_obj) = certs_json.get("app_cert").and_then(|v| v.as_object()) {
            if let (Some(cert_pem), Some(key_pem)) = (
                app_obj.get("cert_pem").and_then(|v| v.as_str()),
                app_obj.get("key_pem").and_then(|v| v.as_str()),
            ) {
                let domain = app_obj
                    .get("wildcard_domain")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                match parse_certified_key(cert_pem, key_pem) {
                    Ok(ck) => {
                        info!("Loaded app cert for {}", domain);
                        new_certs.insert(domain.to_string(), Arc::new(ck));
                    }
                    Err(e) => {
                        warn!("Failed to parse app cert: {e}");
                    }
                }
            }
        }

        // Parse global cert (*.{base})
        if let Some(global_obj) = certs_json.get("global_cert").and_then(|v| v.as_object()) {
            if let (Some(cert_pem), Some(key_pem)) = (
                global_obj.get("cert_pem").and_then(|v| v.as_str()),
                global_obj.get("key_pem").and_then(|v| v.as_str()),
            ) {
                let domain = global_obj
                    .get("wildcard_domain")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                match parse_certified_key(cert_pem, key_pem) {
                    Ok(ck) => {
                        info!("Loaded global cert for {}", domain);
                        new_certs.insert(domain.to_string(), Arc::new(ck));
                    }
                    Err(e) => {
                        warn!("Failed to parse global cert: {e}");
                    }
                }
            }
        }

        if new_certs.is_empty() {
            anyhow::bail!("No certificates loaded from HomeRoute");
        }

        self.resolver.replace_all(new_certs);
        info!("Agent SNI resolver updated");
        Ok(())
    }

    /// Internal: run the TLS server loop.
    async fn run_server(&self) -> Result<()> {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let mut server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(self.resolver.clone());
        server_config.alpn_protocols = vec![b"http/1.1".to_vec()];

        let tls_acceptor = TlsAcceptor::from(Arc::new(server_config));
        let listener = TcpListener::bind("0.0.0.0:443").await?;
        info!("Agent HTTPS proxy listening on 0.0.0.0:443");

        loop {
            let (tcp_stream, peer_addr) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    warn!("TCP accept error: {e}");
                    continue;
                }
            };

            let acceptor = tls_acceptor.clone();
            let routes = Arc::clone(&self.routes);
            let auth_cache = Arc::clone(&self.auth_cache);
            let homeroute_url = self.homeroute_url.clone();

            tokio::spawn(async move {
                let tls_stream = match acceptor.accept(tcp_stream).await {
                    Ok(s) => s,
                    Err(e) => {
                        debug!("TLS handshake failed from {}: {e}", peer_addr);
                        return;
                    }
                };

                let io = TokioIo::new(tls_stream);
                let service = service_fn(move |req: Request<Incoming>| {
                    let routes = Arc::clone(&routes);
                    let auth_cache = Arc::clone(&auth_cache);
                    let homeroute_url = homeroute_url.clone();
                    async move {
                        handle_request(req, peer_addr, &routes, &auth_cache, &homeroute_url).await
                    }
                });

                if let Err(e) = ServerBuilder::new()
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
                        debug!("HTTP serve error from {}: {e}", peer_addr);
                    }
                }
            });
        }
    }
}

// ── Request handling ───────────────────────────────────────────────

async fn handle_request(
    mut req: Request<Incoming>,
    peer_addr: SocketAddr,
    routes: &RwLock<HashMap<String, LocalRoute>>,
    auth_cache: &RwLock<HashMap<String, (Instant, AuthResult)>>,
    homeroute_url: &str,
) -> Result<Response<BoxBody>, hyper::Error> {
    let host = req
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let domain = host.split(':').next().unwrap_or(&host);

    // Look up route
    let route = {
        let routes = routes.read().unwrap();
        routes.get(domain).cloned()
    };
    let route = match route {
        Some(r) => r,
        None => {
            debug!("No route for host: {}", domain);
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(full_body(format!("Domain not configured: {}", domain)))
                .unwrap());
        }
    };

    // Forward-auth if required
    if route.auth_required {
        match check_auth(&req, &route, auth_cache, homeroute_url).await {
            AuthCheckResult::Ok { user, groups } => {
                if let Ok(v) = hyper::header::HeaderValue::from_str(&user) {
                    req.headers_mut().insert("X-Forwarded-User", v);
                }
                if let Ok(v) = hyper::header::HeaderValue::from_str(&groups) {
                    req.headers_mut().insert("X-Forwarded-Groups", v);
                }
            }
            AuthCheckResult::Redirect(url) => {
                return Ok(Response::builder()
                    .status(StatusCode::FOUND)
                    .header("Location", &url)
                    .body(empty_body())
                    .unwrap());
            }
            AuthCheckResult::Forbidden => {
                return Ok(Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .body(full_body("Forbidden"))
                    .unwrap());
            }
            AuthCheckResult::Error(msg) => {
                warn!("Forward-auth error: {msg}");
                // Fall through to allow request (fail-open on auth service unavailability)
            }
        }
    }

    // Set forwarding headers
    let headers = req.headers_mut();
    if let Ok(v) = hyper::header::HeaderValue::from_str(&host) {
        headers.insert("X-Forwarded-Host", v);
    }
    if let Ok(v) = hyper::header::HeaderValue::from_str(&peer_addr.ip().to_string()) {
        headers.insert("X-Forwarded-For", v.clone());
        headers.insert("X-Real-IP", v);
    }
    headers.insert(
        "X-Forwarded-Proto",
        hyper::header::HeaderValue::from_static("https"),
    );

    // Check for WebSocket upgrade
    if is_websocket_upgrade(&req) {
        debug!("WebSocket upgrade for {} -> localhost:{}", domain, route.target_port);
        return handle_websocket_upgrade(req, route.target_port).await;
    }

    // Regular HTTP proxy to localhost:{port}
    proxy_http(req, route.target_port).await
}

// ── HTTP proxy ─────────────────────────────────────────────────────

async fn proxy_http(
    mut req: Request<Incoming>,
    target_port: u16,
) -> Result<Response<BoxBody>, hyper::Error> {
    // Remove hop-by-hop headers
    req.headers_mut().remove("connection");
    req.headers_mut().remove("upgrade");

    // Build target URI
    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.to_string())
        .unwrap_or_else(|| "/".to_string());
    let target_uri: hyper::Uri = format!("http://127.0.0.1:{}{}", target_port, path)
        .parse()
        .unwrap_or_else(|_| "/".parse().unwrap());
    *req.uri_mut() = target_uri;

    let backend_addr = format!("127.0.0.1:{}", target_port);
    let tcp_stream = match TcpStream::connect(&backend_addr).await {
        Ok(s) => s,
        Err(e) => {
            warn!("Backend connect failed ({}): {e}", backend_addr);
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("Backend unavailable: {e}")))
                .unwrap());
        }
    };

    let io = TokioIo::new(tcp_stream);
    let (mut sender, conn) = hyper::client::conn::http1::Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .handshake(io)
        .await
        .map_err(|e| {
            warn!("Backend handshake failed: {e}");
            e
        })?;

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            let msg = e.to_string();
            if !msg.contains("connection closed") && !msg.contains("not connected") {
                debug!("Backend connection error: {e}");
            }
        }
    });

    let resp = sender.send_request(req).await?;

    // Convert Incoming body to BoxBody
    Ok(resp.map(|b| b.boxed()))
}

// ── WebSocket upgrade ──────────────────────────────────────────────

fn is_websocket_upgrade<T>(req: &Request<T>) -> bool {
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

async fn handle_websocket_upgrade(
    mut req: Request<Incoming>,
    target_port: u16,
) -> Result<Response<BoxBody>, hyper::Error> {
    use tokio::io::AsyncWriteExt;

    let client_upgrade = hyper::upgrade::on(&mut req);

    let backend_addr = format!("127.0.0.1:{}", target_port);
    let tcp_stream = match TcpStream::connect(&backend_addr).await {
        Ok(s) => s,
        Err(e) => {
            warn!("WS backend connect failed ({}): {e}", backend_addr);
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("Backend unavailable: {e}")))
                .unwrap());
        }
    };

    let io = TokioIo::new(tcp_stream);

    let (mut sender, conn) = hyper::client::conn::http1::Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .handshake(io)
        .await
        .map_err(|e| {
            warn!("WS backend handshake failed: {e}");
            e
        })?;

    tokio::spawn(async move {
        if let Err(e) = conn.with_upgrades().await {
            let msg = e.to_string();
            if !msg.contains("connection closed") && !msg.contains("not connected") {
                debug!("WS backend connection error: {e}");
            }
        }
    });

    // Build the target URI (path only)
    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.to_string())
        .unwrap_or_else(|| "/".to_string());
    let target_uri: hyper::Uri = path.parse().unwrap_or_else(|_| "/".parse().unwrap());
    *req.uri_mut() = target_uri;

    let backend_response = match sender.send_request(req).await {
        Ok(r) => r,
        Err(e) => {
            warn!("WS backend request failed: {e}");
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body("WebSocket backend error"))
                .unwrap());
        }
    };

    if backend_response.status() != StatusCode::SWITCHING_PROTOCOLS {
        warn!(
            "Backend did not upgrade WebSocket, status: {}",
            backend_response.status()
        );
        return Ok(backend_response.map(|b| b.boxed()));
    }

    info!("WebSocket upgrade successful to {}", backend_addr);

    let mut response_builder = Response::builder().status(StatusCode::SWITCHING_PROTOCOLS);
    for (name, value) in backend_response.headers() {
        response_builder = response_builder.header(name, value);
    }

    let backend_upgrade = hyper::upgrade::on(backend_response);

    let client_response = response_builder.body(empty_body()).unwrap();

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
    });

    Ok(client_response)
}

// ── Forward-auth ───────────────────────────────────────────────────

enum AuthCheckResult {
    Ok { user: String, groups: String },
    Redirect(String),
    Forbidden,
    Error(String),
}

const AUTH_CACHE_TTL_SECS: u64 = 30;

async fn check_auth<T>(
    req: &Request<T>,
    route: &LocalRoute,
    auth_cache: &RwLock<HashMap<String, (Instant, AuthResult)>>,
    homeroute_url: &str,
) -> AuthCheckResult {
    // Extract auth_session cookie
    let session_id = req
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies
                .split(';')
                .find_map(|c| c.trim().strip_prefix("auth_session="))
        })
        .unwrap_or("")
        .to_string();

    if session_id.is_empty() {
        // No session -> redirect to login
        let host = req
            .headers()
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let uri = req
            .uri()
            .path_and_query()
            .map(|pq| pq.to_string())
            .unwrap_or_else(|| "/".to_string());
        let redirect = format!(
            "{}/api/auth/login?redirect_url=https://{}{}",
            homeroute_url, host, uri
        );
        return AuthCheckResult::Redirect(redirect);
    }

    // Check cache
    {
        let cache = auth_cache.read().unwrap();
        if let Some((ts, result)) = cache.get(&session_id) {
            if ts.elapsed().as_secs() < AUTH_CACHE_TTL_SECS {
                return match result {
                    AuthResult::Ok { user, groups } => AuthCheckResult::Ok {
                        user: user.clone(),
                        groups: groups.clone(),
                    },
                    AuthResult::Forbidden => AuthCheckResult::Forbidden,
                };
            }
        }
    }

    // Cache miss -> call HomeRoute forward-auth endpoint
    let host = req
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let uri = req
        .uri()
        .path_and_query()
        .map(|pq| pq.to_string())
        .unwrap_or_else(|| "/".to_string());
    let groups_param = route.allowed_groups.join(",");

    let url = format!(
        "{}/api/auth/forward-check?host={}&uri={}&groups={}",
        homeroute_url,
        urlencoded(host),
        urlencoded(&uri),
        urlencoded(&groups_param)
    );

    let client = reqwest::Client::new();
    let resp = match client
        .get(&url)
        .header("Cookie", format!("auth_session={}", session_id))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return AuthCheckResult::Error(format!("Forward-auth request failed: {e}"));
        }
    };

    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .await
        .unwrap_or(serde_json::Value::Null);

    match status.as_u16() {
        200 => {
            let user = body
                .get("user")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let groups = body
                .get("groups")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // Cache success
            {
                let mut cache = auth_cache.write().unwrap();
                cache.insert(
                    session_id,
                    (Instant::now(), AuthResult::Ok { user: user.clone(), groups: groups.clone() }),
                );
            }
            AuthCheckResult::Ok { user, groups }
        }
        401 => {
            let login_url = body
                .get("login_url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if login_url.is_empty() {
                let redirect = format!(
                    "{}/api/auth/login?redirect_url=https://{}{}",
                    homeroute_url, host, uri
                );
                AuthCheckResult::Redirect(redirect)
            } else {
                AuthCheckResult::Redirect(login_url)
            }
        }
        403 => {
            // Cache forbidden
            {
                let mut cache = auth_cache.write().unwrap();
                cache.insert(session_id, (Instant::now(), AuthResult::Forbidden));
            }
            AuthCheckResult::Forbidden
        }
        _ => {
            AuthCheckResult::Error(format!("Unexpected forward-auth status: {}", status))
        }
    }
}

/// Minimal URL-encoding for query parameter values.
fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '&' => out.push_str("%26"),
            '=' => out.push_str("%3D"),
            '%' => out.push_str("%25"),
            '+' => out.push_str("%2B"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            _ => out.push(c),
        }
    }
    out
}

// ── Certificate loading ────────────────────────────────────────────

/// Pull certificates from the central HomeRoute API.
async fn pull_certs(
    homeroute_url: &str,
    agent_token: &str,
) -> Result<serde_json::Value> {
    let url = format!("{}/api/applications/agents/certs", homeroute_url);
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", agent_token))
        .send()
        .await
        .context("Failed to request certs from HomeRoute")?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "Cert pull failed: HTTP {} - {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }

    resp.json()
        .await
        .context("Failed to parse cert response JSON")
}

/// Parse PEM cert + key into a rustls CertifiedKey.
fn parse_certified_key(cert_pem: &str, key_pem: &str) -> Result<CertifiedKey> {
    let certs: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut BufReader::new(cert_pem.as_bytes()))
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse certificate PEM")?;

    if certs.is_empty() {
        anyhow::bail!("No certificates found in PEM data");
    }

    let key: PrivateKeyDer<'static> =
        rustls_pemfile::private_key(&mut BufReader::new(key_pem.as_bytes()))
            .context("Failed to parse private key PEM")?
            .ok_or_else(|| anyhow::anyhow!("No private key found in PEM data"))?;

    let signing_key = rustls::crypto::ring::sign::any_supported_type(&key)
        .map_err(|e| anyhow::anyhow!("Failed to parse signing key: {e}"))?;

    Ok(CertifiedKey::new(certs, signing_key))
}
