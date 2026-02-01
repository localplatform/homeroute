use crate::users::UserInfo;
use crate::AuthService;
use axum::{
    extract::{FromRequestParts, Request},
    http::{StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use axum_extra::extract::CookieJar;
use serde_json::json;
use std::sync::Arc;

/// Extracteur axum : utilisateur authentifié (optionnel)
///
/// Peuple `Option<AuthUser>` dans les handlers.
#[derive(Debug, Clone)]
pub struct AuthUser(pub UserInfo);

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    Arc<AuthService>: FromRequestParts<S, Rejection = std::convert::Infallible>,
{
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Extraire le cookie de session
        let jar = CookieJar::from_headers(&parts.headers);
        let session_id = jar
            .get("auth_session")
            .map(|c| c.value().to_string());

        let Some(session_id) = session_id else {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({ "success": false, "error": "Non authentifie" })),
            ));
        };

        // Récupérer AuthService depuis les extensions
        let auth = parts
            .extensions
            .get::<Arc<AuthService>>()
            .cloned()
            .ok_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "success": false, "error": "Auth service unavailable" })),
            ))?;

        // Valider la session
        let session = auth
            .sessions
            .validate(&session_id)
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "success": false, "error": "Session validation error" })),
                )
            })?
            .ok_or((
                StatusCode::UNAUTHORIZED,
                Json(json!({ "success": false, "error": "Session expiree" })),
            ))?;

        // Récupérer l'utilisateur
        let user = auth.users.get(&session.user_id).ok_or((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "success": false, "error": "Utilisateur non trouve" })),
        ))?;

        if user.disabled {
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({ "success": false, "error": "Compte desactive" })),
            ));
        }

        Ok(AuthUser(user))
    }
}

/// Extracteur axum : utilisateur admin requis
#[derive(Debug, Clone)]
pub struct AdminUser(pub UserInfo);

impl<S> FromRequestParts<S> for AdminUser
where
    S: Send + Sync,
    Arc<AuthService>: FromRequestParts<S, Rejection = std::convert::Infallible>,
{
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let AuthUser(user) = AuthUser::from_request_parts(parts, state).await?;

        if !user.groups.contains(&"admins".to_string()) {
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({ "success": false, "error": "Acces administrateur requis" })),
            ));
        }

        Ok(AdminUser(user))
    }
}

/// Middleware axum : injecte AuthService dans les extensions de la requête
pub async fn inject_auth_service(
    request: Request,
    next: Next,
) -> Response {
    next.run(request).await
}
