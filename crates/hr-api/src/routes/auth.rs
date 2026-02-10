use axum::{
    extract::State,
    extract::Path,
    http::{header, HeaderMap},
    routing::{delete, get, post},
    Json, Router,
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/check", get(check))
        .route("/forward-check", get(forward_check))
        .route("/me", get(me))
        .route("/sessions", get(list_sessions))
        .route("/sessions/{id}", delete(revoke_session))
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
    #[serde(default)]
    remember_me: bool,
}

fn cookie_domain(headers: &HeaderMap, base_domain: &str) -> Option<String> {
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    let is_secure = proto == "https";
    let is_real_domain = base_domain != "localhost" && host.contains(base_domain);
    if is_real_domain && is_secure {
        Some(format!(".{}", base_domain))
    } else {
        None
    }
}

fn build_set_cookie(session_id: &str, max_age_secs: Option<i64>, headers: &HeaderMap, base_domain: &str) -> String {
    let domain = cookie_domain(headers, base_domain);
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    let is_secure = proto == "https";

    let mut parts = vec![
        format!("auth_session={}", session_id),
        "HttpOnly".to_string(),
        "SameSite=Lax".to_string(),
        "Path=/".to_string(),
    ];
    if is_secure {
        parts.push("Secure".to_string());
    }
    if let Some(d) = domain {
        parts.push(format!("Domain={}", d));
    }
    if let Some(age) = max_age_secs {
        parts.push(format!("Max-Age={}", age));
    }
    parts.join("; ")
}

fn clear_cookie(headers: &HeaderMap, base_domain: &str) -> String {
    build_set_cookie("deleted", Some(0), headers, base_domain)
}

async fn login(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> (axum::http::StatusCode, [(header::HeaderName, String); 1], Json<Value>) {
    let username = body.username.to_lowercase();

    if username.is_empty() || body.password.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            [(header::SET_COOKIE, String::new())],
            Json(json!({"success": false, "error": "Nom d'utilisateur et mot de passe requis"})),
        );
    }

    let user = match state.auth.users.get_with_password(&username) {
        Some(u) => u,
        None => {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                [(header::SET_COOKIE, String::new())],
                Json(json!({"success": false, "error": "Identifiants invalides"})),
            );
        }
    };

    if user.disabled {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            [(header::SET_COOKIE, String::new())],
            Json(json!({"success": false, "error": "Compte desactive"})),
        );
    }

    if !hr_auth::users::verify_password(&body.password, &user.password_hash) {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            [(header::SET_COOKIE, String::new())],
            Json(json!({"success": false, "error": "Identifiants invalides"})),
        );
    }

    let ip = headers.get("x-real-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|v| v.to_str().ok());
    let ua = headers.get("user-agent").and_then(|v| v.to_str().ok());

    let (session_id, expires_at) = match state.auth.sessions.create(
        &username, ip, ua, body.remember_me,
    ) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("Session creation failed: {}", e);
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                [(header::SET_COOKIE, String::new())],
                Json(json!({"success": false, "error": "Erreur lors de la connexion"})),
            );
        }
    };

    state.auth.users.update_last_login(&username);

    let max_age = if body.remember_me { Some(30 * 24 * 60 * 60) } else { None };
    let cookie = build_set_cookie(&session_id, max_age, &headers, &state.auth.base_domain);

    (
        axum::http::StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(json!({
            "success": true,
            "user": {
                "username": user.username,
                "displayname": user.displayname,
                "email": user.email,
                "groups": user.groups
            },
            "expires_at": expires_at
        })),
    )
}

async fn logout(
    State(state): State<ApiState>,
    headers: HeaderMap,
    jar: CookieJar,
) -> ([(header::HeaderName, String); 1], Json<Value>) {
    if let Some(cookie) = jar.get("auth_session") {
        let _ = state.auth.sessions.delete(cookie.value());
    }

    let base = &state.auth.base_domain;
    let clear = clear_cookie(&headers, base);

    (
        [(header::SET_COOKIE, clear)],
        Json(json!({
            "success": true,
            "logoutUrl": format!("https://auth.{}/logout", base)
        })),
    )
}

async fn check(
    State(state): State<ApiState>,
    headers: HeaderMap,
    jar: CookieJar,
) -> (axum::http::StatusCode, [(header::HeaderName, String); 1], Json<Value>) {
    let session_id = match jar.get("auth_session") {
        Some(c) => c.value().to_string(),
        None => {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                [(header::SET_COOKIE, String::new())],
                Json(json!({"success": false, "authenticated": false})),
            );
        }
    };

    match state.auth.sessions.validate(&session_id) {
        Ok(Some(session)) => (
            axum::http::StatusCode::OK,
            [(header::SET_COOKIE, String::new())],
            Json(json!({"success": true, "authenticated": true, "user_id": session.user_id})),
        ),
        _ => {
            let clear = clear_cookie(&headers, &state.auth.base_domain);
            (
                axum::http::StatusCode::UNAUTHORIZED,
                [(header::SET_COOKIE, clear)],
                Json(json!({"success": false, "authenticated": false})),
            )
        }
    }
}

async fn me(
    State(state): State<ApiState>,
    headers: HeaderMap,
    jar: CookieJar,
) -> ([(header::HeaderName, String); 1], Json<Value>) {
    let base = &state.auth.base_domain;
    let auth_url = format!("https://auth.{}", base);

    let session_id = match jar.get("auth_session") {
        Some(c) => c.value().to_string(),
        None => {
            return (
                [(header::SET_COOKIE, String::new())],
                Json(json!({"success": false, "user": null, "authUrl": auth_url})),
            );
        }
    };

    let session = match state.auth.sessions.validate(&session_id) {
        Ok(Some(s)) => s,
        _ => {
            let clear = clear_cookie(&headers, base);
            return (
                [(header::SET_COOKIE, clear)],
                Json(json!({"success": false, "user": null, "error": "Session expiree", "authUrl": auth_url})),
            );
        }
    };

    let user = match state.auth.users.get(&session.user_id) {
        Some(u) => u,
        None => {
            let _ = state.auth.sessions.delete(&session_id);
            let clear = clear_cookie(&headers, base);
            return (
                [(header::SET_COOKIE, clear)],
                Json(json!({"success": false, "user": null, "error": "Utilisateur non trouve", "authUrl": auth_url})),
            );
        }
    };

    let is_admin = user.groups.contains(&"admins".to_string());

    // Refresh cookie domain on every successful /me call
    let remaining_ms = session.expires_at - chrono::Utc::now().timestamp_millis();
    let remaining_secs = (remaining_ms / 1000).max(0);
    let cookie = build_set_cookie(&session_id, Some(remaining_secs), &headers, base);

    (
        [(header::SET_COOKIE, cookie)],
        Json(json!({
            "success": true,
            "user": {
                "username": user.username,
                "displayName": user.displayname,
                "email": user.email,
                "groups": user.groups,
                "isAdmin": is_admin
            },
            "session": {
                "created_at": session.created_at,
                "expires_at": session.expires_at,
                "ip_address": session.ip_address
            },
            "authMethod": "session"
        })),
    )
}

async fn list_sessions(
    State(state): State<ApiState>,
    jar: CookieJar,
) -> (axum::http::StatusCode, Json<Value>) {
    let session_id = match jar.get("auth_session") {
        Some(c) => c.value().to_string(),
        None => return (axum::http::StatusCode::UNAUTHORIZED, Json(json!({"success": false, "error": "Non authentifie"}))),
    };

    let session = match state.auth.sessions.validate(&session_id) {
        Ok(Some(s)) => s,
        _ => return (axum::http::StatusCode::UNAUTHORIZED, Json(json!({"success": false, "error": "Session expiree"}))),
    };

    let sessions = state.auth.sessions.get_by_user(&session.user_id).unwrap_or_default();

    let sessions_json: Vec<Value> = sessions
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "current": s.id == session_id,
                "ip_address": s.ip_address,
                "user_agent": s.user_agent,
                "created_at": s.created_at,
                "last_activity": s.last_activity,
                "remember_me": s.remember_me
            })
        })
        .collect();

    (axum::http::StatusCode::OK, Json(json!({"success": true, "sessions": sessions_json})))
}

async fn revoke_session(
    State(state): State<ApiState>,
    jar: CookieJar,
    Path(target_id): Path<String>,
) -> (axum::http::StatusCode, Json<Value>) {
    let session_id = match jar.get("auth_session") {
        Some(c) => c.value().to_string(),
        None => return (axum::http::StatusCode::UNAUTHORIZED, Json(json!({"success": false, "error": "Non authentifie"}))),
    };

    if target_id == session_id {
        return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"success": false, "error": "Utilisez /logout pour deconnecter la session actuelle"})));
    }

    let session = match state.auth.sessions.validate(&session_id) {
        Ok(Some(s)) => s,
        _ => return (axum::http::StatusCode::UNAUTHORIZED, Json(json!({"success": false, "error": "Session expiree"}))),
    };

    let target = match state.auth.sessions.get(&target_id) {
        Ok(Some(s)) => s,
        _ => return (axum::http::StatusCode::NOT_FOUND, Json(json!({"success": false, "error": "Session non trouvee"}))),
    };

    if target.user_id != session.user_id {
        return (axum::http::StatusCode::NOT_FOUND, Json(json!({"success": false, "error": "Session non trouvee"})));
    }

    let _ = state.auth.sessions.delete(&target_id);

    (axum::http::StatusCode::OK, Json(json!({"success": true})))
}

/// Query parameters for forward-check (used by agent proxies).
#[derive(Deserialize, Default)]
struct ForwardCheckQuery {
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    groups: Option<String>,
}

/// Forward-auth endpoint for agent reverse proxies.
/// Accepts query params: host, uri, groups (comma-separated) — or X-Forwarded-* headers.
/// Returns 200 + user/groups on success, 401 + login_url on unauthenticated, 403 on forbidden.
async fn forward_check(
    State(state): State<ApiState>,
    axum::extract::Query(query): axum::extract::Query<ForwardCheckQuery>,
    headers: HeaderMap,
) -> (axum::http::StatusCode, Json<Value>) {
    use hr_auth::forward_auth::{check_forward_auth, ForwardAuthResult};

    // Query params take precedence over headers (agent use case)
    let forwarded_host = query.host.as_deref()
        .or_else(|| headers.get("x-forwarded-host").and_then(|v| v.to_str().ok()))
        .unwrap_or("");
    let forwarded_uri = query.uri.as_deref()
        .or_else(|| headers.get("x-forwarded-uri").and_then(|v| v.to_str().ok()))
        .unwrap_or("/");
    let forwarded_proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("https");

    let allowed_groups: Vec<String> = query.groups.as_deref()
        .map(|g| g.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    let cookie_value = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                let c = c.trim();
                c.strip_prefix("auth_session=")
            })
        });

    match check_forward_auth(&state.auth, cookie_value, forwarded_host, forwarded_uri, forwarded_proto, &allowed_groups) {
        ForwardAuthResult::Success { user } => {
            let groups = user.groups.join(",");
            (axum::http::StatusCode::OK, Json(json!({"user": user.username, "groups": groups})))
        }
        ForwardAuthResult::Unauthorized { login_url } => {
            (axum::http::StatusCode::UNAUTHORIZED, Json(json!({"login_url": login_url})))
        }
        ForwardAuthResult::Forbidden { message } => {
            (axum::http::StatusCode::FORBIDDEN, Json(json!({"message": message})))
        }
    }
}
