use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/overview", get(overview))
        .route("/timeseries", get(timeseries))
        .route("/by-device", get(by_device))
        .route("/by-endpoint", get(by_endpoint))
        .route("/by-application", get(by_application))
        .route("/device/{mac}", get(device_detail))
        .route("/dns/top-domains", get(dns_top_domains))
        .route("/dns/by-category", get(dns_by_category))
}

#[derive(Deserialize)]
struct TimeRangeQuery {
    #[serde(rename = "timeRange", default = "default_time_range")]
    time_range: String,
    #[serde(default)]
    limit: Option<i64>,
}

fn default_time_range() -> String {
    "24h".to_string()
}

#[derive(Deserialize)]
struct TimeseriesQuery {
    #[serde(default = "default_metric")]
    metric: String,
    #[serde(default = "default_granularity")]
    granularity: String,
    #[serde(rename = "timeRange", default = "default_time_range")]
    time_range: String,
}

fn default_metric() -> String {
    "requests".to_string()
}
fn default_granularity() -> String {
    "hour".to_string()
}

async fn overview(
    State(state): State<ApiState>,
    Query(query): Query<TimeRangeQuery>,
) -> Json<Value> {
    match hr_analytics::query::get_overview(&state.analytics, &query.time_range) {
        Ok(data) => Json(json!({ "success": true, "data": data })),
        Err(e) => Json(json!({ "success": false, "error": e.to_string() })),
    }
}

async fn timeseries(
    State(state): State<ApiState>,
    Query(query): Query<TimeseriesQuery>,
) -> Json<Value> {
    match hr_analytics::query::get_timeseries(
        &state.analytics,
        &query.metric,
        &query.granularity,
        &query.time_range,
    ) {
        Ok(data) => Json(json!({ "success": true, "data": data })),
        Err(e) => Json(json!({ "success": false, "error": e.to_string() })),
    }
}

async fn by_device(
    State(state): State<ApiState>,
    Query(query): Query<TimeRangeQuery>,
) -> Json<Value> {
    let limit = query.limit.unwrap_or(10);
    match hr_analytics::query::get_top_devices(&state.analytics, &query.time_range, limit) {
        Ok(data) => Json(json!({ "success": true, "data": data })),
        Err(e) => Json(json!({ "success": false, "error": e.to_string() })),
    }
}

async fn by_endpoint(
    State(state): State<ApiState>,
    Query(query): Query<TimeRangeQuery>,
) -> Json<Value> {
    let limit = query.limit.unwrap_or(10);
    match hr_analytics::query::get_top_endpoints(&state.analytics, &query.time_range, limit) {
        Ok(data) => Json(json!({ "success": true, "data": data })),
        Err(e) => Json(json!({ "success": false, "error": e.to_string() })),
    }
}

async fn by_application(
    State(state): State<ApiState>,
    Query(query): Query<TimeRangeQuery>,
) -> Json<Value> {
    let limit = query.limit.unwrap_or(20);
    match hr_analytics::query::get_top_applications(&state.analytics, &query.time_range, limit) {
        Ok(data) => Json(json!({ "success": true, "data": data })),
        Err(e) => Json(json!({ "success": false, "error": e.to_string() })),
    }
}

async fn device_detail(
    State(state): State<ApiState>,
    axum::extract::Path(mac): axum::extract::Path<String>,
    Query(query): Query<TimeRangeQuery>,
) -> Json<Value> {
    match hr_analytics::query::get_device_detail(&state.analytics, &mac, &query.time_range) {
        Ok(data) => Json(json!({ "success": true, "data": data })),
        Err(e) => Json(json!({ "success": false, "error": e.to_string() })),
    }
}

async fn dns_top_domains(
    State(state): State<ApiState>,
    Query(query): Query<TimeRangeQuery>,
) -> Json<Value> {
    let limit = query.limit.unwrap_or(20);
    match hr_analytics::query::get_dns_top_domains(&state.analytics, &query.time_range, limit) {
        Ok(data) => Json(json!({ "success": true, "data": data })),
        Err(e) => Json(json!({ "success": false, "error": e.to_string() })),
    }
}

async fn dns_by_category(
    State(state): State<ApiState>,
    Query(query): Query<TimeRangeQuery>,
) -> Json<Value> {
    let limit = query.limit.unwrap_or(20);
    match hr_analytics::query::get_dns_by_category(&state.analytics, &query.time_range, limit) {
        Ok(data) => Json(json!({ "success": true, "data": data })),
        Err(e) => Json(json!({ "success": false, "error": e.to_string() })),
    }
}
