use axum::{extract::State, routing::get, Json, Router};
use serde_json::{json, Value};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new().route("/status", get(status))
}

async fn status(State(state): State<ApiState>) -> Json<Value> {
    let registry = state.service_registry.read().await;

    let mut services: Vec<_> = registry.values().cloned().collect();
    services.sort_by(|a, b| {
        a.priority.cmp(&b.priority).then(a.name.cmp(&b.name))
    });

    Json(json!({
        "success": true,
        "services": services
    }))
}
