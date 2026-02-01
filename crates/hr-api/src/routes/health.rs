use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new().route("/health", get(health))
}

async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}
