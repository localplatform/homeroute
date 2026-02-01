use axum::{
    extract::{Path, State},
    routing::{get, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/", get(list_users).post(create_user))
        .route("/{username}", get(get_user).put(update_user).delete(delete_user))
        .route("/{username}/password", put(change_password))
        .route("/groups", get(list_groups))
}

async fn list_users(State(state): State<ApiState>) -> Json<Value> {
    let users = state.auth.users.get_all();
    Json(json!(users))
}

async fn get_user(State(state): State<ApiState>, Path(username): Path<String>) -> (axum::http::StatusCode, Json<Value>) {
    match state.auth.users.get(&username) {
        Some(user) => (axum::http::StatusCode::OK, Json(json!(user))),
        None => (axum::http::StatusCode::NOT_FOUND, Json(json!({"success": false, "error": "Utilisateur non trouve"}))),
    }
}

#[derive(Deserialize)]
struct CreateUserRequest {
    username: String,
    password: String,
    #[serde(default)]
    displayname: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    groups: Vec<String>,
}

async fn create_user(State(state): State<ApiState>, Json(body): Json<CreateUserRequest>) -> Json<Value> {
    let result = state.auth.users.create(
        &body.username.to_lowercase(),
        &body.password,
        body.displayname.as_deref(),
        body.email.as_deref(),
        body.groups,
    );
    Json(json!(result))
}

async fn update_user(
    State(state): State<ApiState>,
    Path(username): Path<String>,
    Json(updates): Json<hr_auth::users::UserUpdates>,
) -> Json<Value> {
    let result = state.auth.users.update(&username, &updates);
    Json(json!(result))
}

async fn delete_user(State(state): State<ApiState>, Path(username): Path<String>) -> Json<Value> {
    // Also delete all sessions for this user
    let _ = state.auth.sessions.delete_by_user(&username);
    let result = state.auth.users.delete(&username);
    Json(json!(result))
}

#[derive(Deserialize)]
struct ChangePasswordRequest {
    password: String,
}

async fn change_password(
    State(state): State<ApiState>,
    Path(username): Path<String>,
    Json(body): Json<ChangePasswordRequest>,
) -> Json<Value> {
    let result = state.auth.users.change_password(&username, &body.password);
    Json(json!(result))
}

async fn list_groups(State(state): State<ApiState>) -> Json<Value> {
    // Return predefined groups + custom groups from users
    let users = state.auth.users.get_all();
    let mut groups = std::collections::BTreeSet::new();
    groups.insert("admins".to_string());
    groups.insert("users".to_string());
    for user in &users {
        for group in &user.groups {
            groups.insert(group.clone());
        }
    }
    let groups: Vec<Value> = groups.iter().map(|g| {
        json!({"id": g, "name": g, "builtin": g == "admins" || g == "users"})
    }).collect();
    Json(json!(groups))
}
