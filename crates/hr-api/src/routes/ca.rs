use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/status", get(status))
        .route("/init", post(init))
        .route("/root-cert", get(root_cert))
        .route("/certificates", get(list_certificates))
        .route("/issue", post(issue))
        .route("/renew/{id}", post(renew))
        .route("/revoke/{id}", delete(revoke))
        .route("/renewal-candidates", get(renewal_candidates))
}

async fn status(State(state): State<ApiState>) -> Json<Value> {
    Json(json!({
        "success": true,
        "initialized": state.ca.is_initialized()
    }))
}

async fn init(State(state): State<ApiState>) -> Json<Value> {
    match state.ca.init().await {
        Ok(()) => Json(json!({"success": true})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

#[derive(Deserialize)]
struct RootCertQuery {
    #[serde(default = "default_pem")]
    format: String,
}

fn default_pem() -> String {
    "pem".to_string()
}

async fn root_cert(
    State(state): State<ApiState>,
    Query(query): Query<RootCertQuery>,
) -> impl IntoResponse {
    match query.format.as_str() {
        "der" => match state.ca.get_root_cert_der().await {
            Ok(der) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/x-x509-ca-cert"),
                 (header::CONTENT_DISPOSITION, "attachment; filename=\"homeroute-ca.der\"")],
                der,
            ).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": e.to_string()})),
            ).into_response(),
        },
        "crt" | "pem" | _ => match state.ca.get_root_cert_pem().await {
            Ok(pem) => {
                let (content_type, filename) = if query.format == "crt" {
                    ("application/x-x509-ca-cert", "homeroute-ca.crt")
                } else {
                    ("application/x-pem-file", "homeroute-ca.pem")
                };
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, content_type),
                     (header::CONTENT_DISPOSITION, &format!("attachment; filename=\"{}\"", filename))],
                    pem,
                ).into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"success": false, "error": e.to_string()})),
            ).into_response(),
        },
    }
}

async fn list_certificates(State(state): State<ApiState>) -> Json<Value> {
    match state.ca.list_certificates() {
        Ok(certs) => {
            let certs_json: Vec<Value> = certs
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "domains": c.domains,
                        "issued_at": c.issued_at.to_rfc3339(),
                        "expires_at": c.expires_at.to_rfc3339(),
                        "serial_number": c.serial_number,
                        "expired": c.is_expired(),
                        "needs_renewal": c.needs_renewal(30)
                    })
                })
                .collect();
            Json(json!({"success": true, "certificates": certs_json}))
        }
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

#[derive(Deserialize)]
struct IssueRequest {
    domains: Vec<String>,
}

async fn issue(
    State(state): State<ApiState>,
    Json(body): Json<IssueRequest>,
) -> Json<Value> {
    match state.ca.issue_certificate(body.domains).await {
        Ok(cert) => Json(json!({
            "success": true,
            "certificate": {
                "id": cert.id,
                "domains": cert.domains,
                "issued_at": cert.issued_at.to_rfc3339(),
                "expires_at": cert.expires_at.to_rfc3339(),
                "serial_number": cert.serial_number
            }
        })),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn renew(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Json<Value> {
    match state.ca.renew_certificate(&id).await {
        Ok(cert) => Json(json!({
            "success": true,
            "certificate": {
                "id": cert.id,
                "domains": cert.domains,
                "issued_at": cert.issued_at.to_rfc3339(),
                "expires_at": cert.expires_at.to_rfc3339()
            }
        })),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn revoke(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Json<Value> {
    match state.ca.revoke_certificate(&id) {
        Ok(()) => Json(json!({"success": true})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn renewal_candidates(State(state): State<ApiState>) -> Json<Value> {
    match state.ca.certificates_needing_renewal() {
        Ok(certs) => {
            let certs_json: Vec<Value> = certs
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "domains": c.domains,
                        "expires_at": c.expires_at.to_rfc3339()
                    })
                })
                .collect();
            Json(json!({"success": true, "certificates": certs_json}))
        }
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}
