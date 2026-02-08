use axum::{
    extract::{Path, State},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde_json::json;

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/overview", get(overview))
        .route("/apps/{app_id}/schema", get(app_schema))
        .route("/apps/{app_id}/tables", get(app_tables))
        .route("/apps/{app_id}/tables/{table_name}", get(app_table))
        .route("/apps/{app_id}/relations", get(app_relations))
        .route("/apps/{app_id}/stats", get(app_stats))
}

async fn overview(State(state): State<ApiState>) -> impl IntoResponse {
    let schemas = state.dataverse_schemas.read().await;
    let apps: Vec<serde_json::Value> = schemas.values().map(|s| {
        json!({
            "appId": s.app_id,
            "slug": s.slug,
            "tables": s.tables.iter().map(|t| json!({
                "name": t.name,
                "slug": t.slug,
                "columnsCount": t.columns.len(),
                "rowsCount": t.row_count,
                "columns": t.columns.iter().map(|c| json!({
                    "name": c.name,
                    "fieldType": c.field_type,
                    "required": c.required,
                    "unique": c.unique,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "relationsCount": s.relations.len(),
            "version": s.version,
            "dbSizeBytes": s.db_size_bytes,
            "lastUpdated": s.last_updated.to_rfc3339(),
        })
    }).collect();

    Json(json!({ "apps": apps }))
}

async fn app_schema(
    State(state): State<ApiState>,
    Path(app_id): Path<String>,
) -> impl IntoResponse {
    let schemas = state.dataverse_schemas.read().await;
    match schemas.get(&app_id) {
        Some(schema) => Json(json!({
            "data": schema,
            "meta": { "app_id": app_id, "version": schema.version, "last_updated": schema.last_updated.to_rfc3339() }
        })).into_response(),
        None => (axum::http::StatusCode::NOT_FOUND, Json(json!({"error": "No schema data for this application"}))).into_response(),
    }
}

async fn app_tables(
    State(state): State<ApiState>,
    Path(app_id): Path<String>,
) -> impl IntoResponse {
    let schemas = state.dataverse_schemas.read().await;
    match schemas.get(&app_id) {
        Some(schema) => Json(json!({
            "tables": schema.tables,
            "meta": { "app_id": app_id }
        })).into_response(),
        None => (axum::http::StatusCode::NOT_FOUND, Json(json!({"error": "No schema data for this application"}))).into_response(),
    }
}

async fn app_table(
    State(state): State<ApiState>,
    Path((app_id, table_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let schemas = state.dataverse_schemas.read().await;
    match schemas.get(&app_id) {
        Some(schema) => {
            match schema.tables.iter().find(|t| t.name == table_name) {
                Some(table) => Json(json!({
                    "table": table,
                    "meta": { "app_id": app_id }
                })).into_response(),
                None => (axum::http::StatusCode::NOT_FOUND, Json(json!({"error": format!("Table '{}' not found", table_name)}))).into_response(),
            }
        }
        None => (axum::http::StatusCode::NOT_FOUND, Json(json!({"error": "No schema data for this application"}))).into_response(),
    }
}

async fn app_relations(
    State(state): State<ApiState>,
    Path(app_id): Path<String>,
) -> impl IntoResponse {
    let schemas = state.dataverse_schemas.read().await;
    match schemas.get(&app_id) {
        Some(schema) => Json(json!({
            "relations": schema.relations,
            "meta": { "app_id": app_id }
        })).into_response(),
        None => (axum::http::StatusCode::NOT_FOUND, Json(json!({"error": "No schema data for this application"}))).into_response(),
    }
}

async fn app_stats(
    State(state): State<ApiState>,
    Path(app_id): Path<String>,
) -> impl IntoResponse {
    let schemas = state.dataverse_schemas.read().await;
    match schemas.get(&app_id) {
        Some(schema) => {
            let total_rows: u64 = schema.tables.iter().map(|t| t.row_count).sum();
            Json(json!({
                "dbSizeBytes": schema.db_size_bytes,
                "tablesCount": schema.tables.len(),
                "relationsCount": schema.relations.len(),
                "totalRows": total_rows,
                "version": schema.version,
                "meta": { "app_id": app_id, "last_updated": schema.last_updated.to_rfc3339() }
            })).into_response()
        }
        None => (axum::http::StatusCode::NOT_FOUND, Json(json!({"error": "No schema data for this application"}))).into_response(),
    }
}
