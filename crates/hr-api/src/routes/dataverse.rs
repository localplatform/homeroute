use axum::{
    body::Body,
    extract::{Path, Query, State},
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use hr_registry::protocol::DataverseQueryRequest;

use crate::state::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/overview", get(overview))
        .route("/apps/{app_id}/schema", get(app_schema))
        .route("/apps/{app_id}/tables", get(app_tables))
        .route("/apps/{app_id}/tables/{table_name}", get(app_table))
        .route("/apps/{app_id}/tables/{table_name}/rows", get(query_rows))
        .route("/apps/{app_id}/tables/{table_name}/rows", post(insert_rows))
        .route("/apps/{app_id}/tables/{table_name}/rows", put(update_rows))
        .route("/apps/{app_id}/tables/{table_name}/rows", delete(delete_rows))
        .route("/apps/{app_id}/tables/{table_name}/count", get(count_rows))
        .route("/apps/{app_id}/relations", get(app_relations))
        .route("/apps/{app_id}/stats", get(app_stats))
        .route("/apps/{app_id}/migrations", get(app_migrations))
        .route("/apps/{app_id}/backup", get(backup_download))
}

// ── Helper ────────────────────────────────────────────────────

async fn proxy_query(state: &ApiState, app_id: &str, query: DataverseQueryRequest) -> impl IntoResponse {
    let Some(registry) = &state.registry else {
        return (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "Registry not available"}))).into_response();
    };
    match registry.dataverse_query(app_id, query).await {
        Ok(data) => Json(json!({ "data": data })).into_response(),
        Err(e) => {
            let msg = e.to_string();
            let status = if msg.contains("not connected") {
                axum::http::StatusCode::SERVICE_UNAVAILABLE
            } else if msg.contains("timeout") {
                axum::http::StatusCode::GATEWAY_TIMEOUT
            } else {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(json!({"error": msg}))).into_response()
        }
    }
}

// ── Existing read-only routes ─────────────────────────────────

async fn overview(
    State(state): State<ApiState>,
) -> impl IntoResponse {
    let schemas = state.dataverse_schemas.read().await;
    let apps: Vec<serde_json::Value> = schemas.values()
        .map(|s| {
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
            "meta": {
                "app_id": app_id,
                "version": schema.version,
                "last_updated": schema.last_updated.to_rfc3339(),
            }
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

// ── Data CRUD routes (proxy to agent) ─────────────────────────

#[derive(Deserialize)]
struct RowsQuery {
    #[serde(default = "default_limit")]
    limit: u64,
    #[serde(default)]
    offset: u64,
    #[serde(default)]
    order_by: Option<String>,
    #[serde(default)]
    order_desc: Option<bool>,
    /// JSON-encoded filters array
    #[serde(default)]
    filters: Option<String>,
}

fn default_limit() -> u64 {
    100
}

async fn query_rows(
    State(state): State<ApiState>,
    Path((app_id, table_name)): Path<(String, String)>,
    Query(params): Query<RowsQuery>,
) -> impl IntoResponse {
    let filters: Vec<serde_json::Value> = params.filters
        .and_then(|f| serde_json::from_str(&f).ok())
        .unwrap_or_default();

    proxy_query(&state, &app_id, DataverseQueryRequest::QueryRows {
        table_name,
        filters,
        limit: params.limit,
        offset: params.offset,
        order_by: params.order_by,
        order_desc: params.order_desc.unwrap_or(false),
    }).await.into_response()
}

#[derive(Deserialize)]
struct InsertBody {
    rows: Vec<serde_json::Value>,
}

async fn insert_rows(
    State(state): State<ApiState>,
    Path((app_id, table_name)): Path<(String, String)>,
    Json(body): Json<InsertBody>,
) -> impl IntoResponse {
    proxy_query(&state, &app_id, DataverseQueryRequest::InsertRows {
        table_name,
        rows: body.rows,
    }).await.into_response()
}

#[derive(Deserialize)]
struct UpdateBody {
    updates: serde_json::Value,
    filters: Vec<serde_json::Value>,
}

async fn update_rows(
    State(state): State<ApiState>,
    Path((app_id, table_name)): Path<(String, String)>,
    Json(body): Json<UpdateBody>,
) -> impl IntoResponse {
    proxy_query(&state, &app_id, DataverseQueryRequest::UpdateRows {
        table_name,
        updates: body.updates,
        filters: body.filters,
    }).await.into_response()
}

#[derive(Deserialize)]
struct DeleteBody {
    filters: Vec<serde_json::Value>,
}

async fn delete_rows(
    State(state): State<ApiState>,
    Path((app_id, table_name)): Path<(String, String)>,
    Json(body): Json<DeleteBody>,
) -> impl IntoResponse {
    proxy_query(&state, &app_id, DataverseQueryRequest::DeleteRows {
        table_name,
        filters: body.filters,
    }).await.into_response()
}

async fn count_rows(
    State(state): State<ApiState>,
    Path((app_id, table_name)): Path<(String, String)>,
    Query(params): Query<RowsQuery>,
) -> impl IntoResponse {
    let filters: Vec<serde_json::Value> = params.filters
        .and_then(|f| serde_json::from_str(&f).ok())
        .unwrap_or_default();

    proxy_query(&state, &app_id, DataverseQueryRequest::CountRows {
        table_name,
        filters,
    }).await.into_response()
}

async fn app_migrations(
    State(state): State<ApiState>,
    Path(app_id): Path<String>,
) -> impl IntoResponse {
    proxy_query(&state, &app_id, DataverseQueryRequest::GetMigrations).await.into_response()
}

// ── Backup route ──────────────────────────────────────────────

async fn backup_download(
    State(state): State<ApiState>,
    Path(app_id): Path<String>,
) -> impl IntoResponse {
    // Look up the app slug and container info
    let Some(registry) = &state.registry else {
        return (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "Registry not available"}))).into_response();
    };

    let apps = registry.list_applications().await;
    let app = match apps.iter().find(|a| a.id == app_id) {
        Some(a) => a,
        None => return (axum::http::StatusCode::NOT_FOUND, Json(json!({"error": "Application not found"}))).into_response(),
    };

    let slug = app.slug.clone();
    let container_name = app.container_name.clone();
    let host_id = app.host_id.clone();

    // Resolve the storage path for the container
    let storage_path = if let Some(cm) = &state.container_manager {
        cm.resolve_storage_path(&host_id).await
    } else {
        "/var/lib/machines".to_string()
    };

    // Only support local containers for now
    if host_id != "local" {
        return (axum::http::StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "Backup only supported for local containers"}))).into_response();
    }

    let db_path = std::path::PathBuf::from(&storage_path)
        .join(&container_name)
        .join("root/workspace/.dataverse/app.db");

    if !db_path.exists() {
        return (axum::http::StatusCode::NOT_FOUND, Json(json!({"error": "No Dataverse database found for this application"}))).into_response();
    }

    // Create a backup copy using sqlite3 .backup to ensure WAL consistency
    let backup_path = std::env::temp_dir().join(format!("dataverse-backup-{}.db", app_id));
    let backup_result = tokio::process::Command::new("sqlite3")
        .arg(&db_path)
        .arg(format!(".backup '{}'", backup_path.display()))
        .output()
        .await;

    let backup_file = match backup_result {
        Ok(output) if output.status.success() => backup_path.clone(),
        _ => {
            // Fallback: direct copy if sqlite3 is not available
            if let Err(e) = tokio::fs::copy(&db_path, &backup_path).await {
                return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("Failed to copy database: {}", e)}))).into_response();
            }
            backup_path.clone()
        }
    };

    // Read the backup file into memory
    let bytes = match tokio::fs::read(&backup_file).await {
        Ok(b) => b,
        Err(e) => {
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("Failed to read backup: {}", e)}))).into_response();
        }
    };

    let body = Body::from(bytes);

    let filename = format!("dataverse-{}.db", slug);

    // Clean up the temp file after a delay (fire and forget)
    let cleanup_path = backup_file;
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let _ = tokio::fs::remove_file(cleanup_path).await;
    });

    axum::http::Response::builder()
        .status(200)
        .header("Content-Type", "application/x-sqlite3")
        .header("Content-Disposition", format!("attachment; filename=\"{}\"", filename))
        .body(body)
        .unwrap()
        .into_response()
}
