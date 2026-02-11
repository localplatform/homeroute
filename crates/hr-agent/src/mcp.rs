//! MCP (Model Context Protocol) stdio server for Dataverse operations.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, RwLock};
use tracing::info;

use hr_dataverse::engine::DataverseEngine;
use hr_dataverse::query::*;
use hr_dataverse::schema::*;
use hr_registry::protocol::{AgentMessage, AppSchemaOverview};

use hr_registry::types::Environment;

use crate::dataverse::LocalDataverse;

/// Shared map for pending schema query responses.
/// The MCP tool registers a oneshot sender here before sending the request,
/// and the main WebSocket loop resolves it when the response arrives.
pub type SchemaQuerySignals =
    Arc<RwLock<HashMap<String, oneshot::Sender<Vec<AppSchemaOverview>>>>>;

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

/// Context for deploy tools (only available in Development environments).
#[derive(Clone)]
pub struct DeployContext {
    pub app_id: String,
    pub api_base_url: String,
    pub environment: Environment,
}

/// Run the MCP stdio server for Dataverse tools.
///
/// When `outbound_tx` and `schema_signals` are provided, the server can
/// send requests to the registry via the WebSocket and wait for responses
/// (used by the `list_other_apps_schemas` tool).
pub async fn run_mcp_server_with_registry(
    outbound_tx: Option<mpsc::Sender<AgentMessage>>,
    schema_signals: Option<SchemaQuerySignals>,
) -> Result<()> {
    info!("Starting MCP Dataverse server");

    let dataverse = LocalDataverse::open()?;
    let engine = dataverse.engine().clone();

    let stdin = io::stdin();
    let stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {}", e),
                        data: None,
                    }),
                };
                writeln!(&stdout, "{}", serde_json::to_string(&resp)?)?;
                continue;
            }
        };

        let id = request.id.clone().unwrap_or(Value::Null);

        let result = match request.method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "hr-dataverse",
                    "version": "0.1.0"
                },
                "instructions": include_str!("mcp_instructions.txt")
            })),
            "notifications/initialized" => {
                // No response needed for notifications
                continue;
            }
            "tools/list" => {
                let tools = get_tool_definitions();
                Ok(json!({ "tools": tools }))
            },
            "tools/call" => {
                let tool_name = request
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let arguments = request
                    .params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(json!({}));

                // Registry-backed tools (async, no engine lock needed)
                if tool_name == "list_other_apps_schemas" {
                    handle_list_other_apps_schemas(
                        outbound_tx.as_ref(),
                        schema_signals.as_ref(),
                    )
                    .await
                } else {
                    // Local Dataverse tools (need engine lock)
                    let engine_guard = engine.lock().await;
                    let res = handle_tool_call(&engine_guard, tool_name, &arguments);
                    drop(engine_guard);
                    res
                }
            }
            _ => Err(format!("Method not found: {}", request.method)),
        };

        let resp = match result {
            Ok(value) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: Some(value),
                error: None,
            },
            Err(e) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32603,
                    message: e,
                    data: None,
                }),
            },
        };

        writeln!(&stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.lock().flush()?;
    }

    Ok(())
}

/// Run the MCP stdio server without registry communication (standalone mode).
pub async fn run_mcp_server() -> Result<()> {
    run_mcp_server_with_registry(None, None).await
}

/// Run the Deploy MCP stdio server (separate from Dataverse).
/// Only exposes deploy, deploy_status, and prod_logs tools.
pub async fn run_deploy_mcp_server(ctx: DeployContext) -> Result<()> {
    info!("Starting MCP Deploy server");

    let stdin = io::stdin();
    let stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {}", e),
                        data: None,
                    }),
                };
                writeln!(&stdout, "{}", serde_json::to_string(&resp)?)?;
                continue;
            }
        };

        let id = request.id.clone().unwrap_or(Value::Null);

        let result = match request.method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "hr-deploy",
                    "version": "0.1.0"
                },
                "instructions": "Deploy tools for pushing builds from development to production containers."
            })),
            "notifications/initialized" => {
                continue;
            }
            "tools/list" => {
                let tools = get_deploy_tool_definitions();
                Ok(json!({ "tools": tools }))
            },
            "tools/call" => {
                let tool_name = request
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let arguments = request
                    .params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(json!({}));
                handle_deploy_tool_call(Some(&ctx), tool_name, &arguments).await
            }
            _ => Err(format!("Method not found: {}", request.method)),
        };

        let resp = match result {
            Ok(value) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: Some(value),
                error: None,
            },
            Err(e) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32603,
                    message: e,
                    data: None,
                }),
            },
        };

        writeln!(&stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.lock().flush()?;
    }

    Ok(())
}

fn get_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "list_tables",
            "description": "List all tables in the Dataverse database with their column counts and row counts.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "describe_table",
            "description": "Get the full schema of a table including all columns and their types.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "table_name": { "type": "string", "description": "Name of the table to describe" }
                },
                "required": ["table_name"]
            }
        }),
        json!({
            "name": "create_table",
            "description": "Create a new table with the specified columns. Each table automatically gets id, created_at, and updated_at columns.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Table name (alphanumeric + underscore)" },
                    "slug": { "type": "string", "description": "URL-friendly slug for the table" },
                    "description": { "type": "string", "description": "Optional table description" },
                    "columns": {
                        "type": "array",
                        "description": "Column definitions",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "field_type": { "type": "string", "enum": ["text", "number", "decimal", "boolean", "date_time", "date", "time", "email", "url", "phone", "currency", "percent", "duration", "json", "uuid", "auto_increment", "choice", "multi_choice", "lookup", "formula"] },
                                "required": { "type": "boolean", "default": false },
                                "unique": { "type": "boolean", "default": false },
                                "default_value": { "type": "string" },
                                "description": { "type": "string" },
                                "choices": { "type": "array", "items": { "type": "string" } }
                            },
                            "required": ["name", "field_type"]
                        }
                    }
                },
                "required": ["name", "slug", "columns"]
            }
        }),
        json!({
            "name": "add_column",
            "description": "Add a new column to an existing table.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "table_name": { "type": "string" },
                    "name": { "type": "string" },
                    "field_type": { "type": "string" },
                    "required": { "type": "boolean", "default": false },
                    "unique": { "type": "boolean", "default": false },
                    "default_value": { "type": "string" }
                },
                "required": ["table_name", "name", "field_type"]
            }
        }),
        json!({
            "name": "remove_column",
            "description": "Remove a column from a table.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "table_name": { "type": "string" },
                    "column_name": { "type": "string" }
                },
                "required": ["table_name", "column_name"]
            }
        }),
        json!({
            "name": "drop_table",
            "description": "Drop (delete) a table and all its data. This action is irreversible.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "table_name": { "type": "string" },
                    "confirm": { "type": "boolean", "description": "Must be true to confirm deletion" }
                },
                "required": ["table_name", "confirm"]
            }
        }),
        json!({
            "name": "query_data",
            "description": "Query rows from a table with optional filters and pagination.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "table_name": { "type": "string" },
                    "filters": { "type": "array", "items": { "type": "object", "properties": { "column": {"type":"string"}, "op": {"type":"string","enum":["eq","ne","gt","lt","gte","lte","like","in","is_null","is_not_null"]}, "value": {} } } },
                    "limit": { "type": "integer", "default": 100 },
                    "offset": { "type": "integer", "default": 0 },
                    "order_by": { "type": "string" },
                    "order_desc": { "type": "boolean", "default": false }
                },
                "required": ["table_name"]
            }
        }),
        json!({
            "name": "insert_data",
            "description": "Insert one or more rows into a table.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "table_name": { "type": "string" },
                    "rows": { "type": "array", "items": { "type": "object" }, "description": "Array of row objects (key=column, value=data)" }
                },
                "required": ["table_name", "rows"]
            }
        }),
        json!({
            "name": "update_data",
            "description": "Update rows in a table matching the given filters.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "table_name": { "type": "string" },
                    "updates": { "type": "object", "description": "Column-value pairs to update" },
                    "filters": { "type": "array", "items": { "type": "object" } }
                },
                "required": ["table_name", "updates", "filters"]
            }
        }),
        json!({
            "name": "delete_data",
            "description": "Delete rows from a table matching the given filters.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "table_name": { "type": "string" },
                    "filters": { "type": "array", "items": { "type": "object" } }
                },
                "required": ["table_name", "filters"]
            }
        }),
        json!({
            "name": "get_schema",
            "description": "Get the full database schema as JSON, including all tables, columns, and relations.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "get_db_info",
            "description": "Get database statistics: file size, table count, total row count.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "create_relation",
            "description": "Create a relation between two tables.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from_table": {"type":"string"}, "from_column": {"type":"string"},
                    "to_table": {"type":"string"}, "to_column": {"type":"string"},
                    "relation_type": {"type":"string","enum":["one_to_many","many_to_many","self_referential"]},
                    "on_delete": {"type":"string","enum":["cascade","set_null","restrict"],"default":"restrict"},
                    "on_update": {"type":"string","enum":["cascade","set_null","restrict"],"default":"cascade"}
                },
                "required": ["from_table","from_column","to_table","to_column","relation_type"]
            }
        }),
        json!({
            "name": "count_rows",
            "description": "Count rows in a table, optionally with filters.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "table_name": { "type": "string" },
                    "filters": { "type": "array", "items": { "type": "object" } }
                },
                "required": ["table_name"]
            }
        }),
        json!({
            "name": "list_other_apps_schemas",
            "description": "List the database schemas (tables, columns, relations) of all other applications in the HomeRoute network. Useful for understanding what data other apps have and how to integrate with them.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
    ]
}

fn handle_tool_call(engine: &DataverseEngine, tool: &str, args: &Value) -> Result<Value, String> {
    let text_result = |text: String| -> Value {
        json!({ "content": [{ "type": "text", "text": text }] })
    };

    match tool {
        "list_tables" => {
            let schema = engine.get_schema().map_err(|e| e.to_string())?;
            let mut tables_info = Vec::new();
            for t in &schema.tables {
                let rows = engine.count_rows(&t.name).unwrap_or(0);
                tables_info.push(json!({
                    "name": t.name,
                    "slug": t.slug,
                    "columns": t.columns.len(),
                    "rows": rows,
                    "description": t.description,
                }));
            }
            Ok(text_result(
                serde_json::to_string_pretty(&tables_info).unwrap(),
            ))
        }

        "describe_table" => {
            let name = args
                .get("table_name")
                .and_then(|v| v.as_str())
                .ok_or("table_name required")?;
            let table = engine
                .get_table(name)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Table '{}' not found", name))?;
            Ok(text_result(serde_json::to_string_pretty(&table).unwrap()))
        }

        "create_table" => {
            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("name required")?
                .to_string();
            let slug = args
                .get("slug")
                .and_then(|v| v.as_str())
                .ok_or("slug required")?
                .to_string();
            let desc = args
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);
            let cols_val = args.get("columns").ok_or("columns required")?;
            let columns: Vec<ColumnDefinition> = serde_json::from_value(cols_val.clone())
                .map_err(|e| format!("Invalid columns: {}", e))?;

            let now = chrono::Utc::now();
            let table = TableDefinition {
                name: name.clone(),
                slug,
                columns,
                description: desc,
                created_at: now,
                updated_at: now,
            };
            let version = engine.create_table(&table).map_err(|e| e.to_string())?;
            Ok(text_result(format!(
                "Table '{}' created (schema version {})",
                name, version
            )))
        }

        "add_column" => {
            let table = args
                .get("table_name")
                .and_then(|v| v.as_str())
                .ok_or("table_name required")?;
            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("name required")?
                .to_string();
            let ft_str = args
                .get("field_type")
                .and_then(|v| v.as_str())
                .ok_or("field_type required")?;
            let field_type: FieldType = serde_json::from_str(&format!("\"{}\"", ft_str))
                .map_err(|_| format!("Invalid field_type: {}", ft_str))?;
            let required = args
                .get("required")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let unique = args
                .get("unique")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let default_value = args
                .get("default_value")
                .and_then(|v| v.as_str())
                .map(String::from);

            let col = ColumnDefinition {
                name: name.clone(),
                field_type,
                required,
                unique,
                default_value,
                description: None,
                choices: vec![],
            };
            let version = engine.add_column(table, &col).map_err(|e| e.to_string())?;
            Ok(text_result(format!(
                "Column '{}' added to '{}' (schema version {})",
                name, table, version
            )))
        }

        "remove_column" => {
            let table = args
                .get("table_name")
                .and_then(|v| v.as_str())
                .ok_or("table_name required")?;
            let col = args
                .get("column_name")
                .and_then(|v| v.as_str())
                .ok_or("column_name required")?;
            let version = engine
                .remove_column(table, col)
                .map_err(|e| e.to_string())?;
            Ok(text_result(format!(
                "Column '{}' removed from '{}' (schema version {})",
                col, table, version
            )))
        }

        "drop_table" => {
            let name = args
                .get("table_name")
                .and_then(|v| v.as_str())
                .ok_or("table_name required")?;
            let confirm = args
                .get("confirm")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !confirm {
                return Err("Set confirm=true to confirm table deletion".to_string());
            }
            let version = engine.drop_table(name).map_err(|e| e.to_string())?;
            Ok(text_result(format!(
                "Table '{}' dropped (schema version {})",
                name, version
            )))
        }

        "query_data" => {
            let table = args
                .get("table_name")
                .and_then(|v| v.as_str())
                .ok_or("table_name required")?;
            let filters: Vec<Filter> = args
                .get("filters")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let pagination = Pagination {
                limit: args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(100),
                offset: args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0),
                order_by: args
                    .get("order_by")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                order_desc: args
                    .get("order_desc")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            };
            let rows = query_rows(engine.connection(), table, &filters, &pagination)
                .map_err(|e| e.to_string())?;
            Ok(text_result(serde_json::to_string_pretty(&rows).unwrap()))
        }

        "insert_data" => {
            let table = args
                .get("table_name")
                .and_then(|v| v.as_str())
                .ok_or("table_name required")?;
            let rows: Vec<Value> = args
                .get("rows")
                .and_then(|v| v.as_array())
                .cloned()
                .ok_or("rows required (array)")?;
            let count =
                insert_rows(engine.connection(), table, &rows).map_err(|e| e.to_string())?;
            Ok(text_result(format!(
                "{} row(s) inserted into '{}'",
                count, table
            )))
        }

        "update_data" => {
            let table = args
                .get("table_name")
                .and_then(|v| v.as_str())
                .ok_or("table_name required")?;
            let updates = args.get("updates").ok_or("updates required")?;
            let filters: Vec<Filter> = args
                .get("filters")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let count = update_rows(engine.connection(), table, updates, &filters)
                .map_err(|e| e.to_string())?;
            Ok(text_result(format!(
                "{} row(s) updated in '{}'",
                count, table
            )))
        }

        "delete_data" => {
            let table = args
                .get("table_name")
                .and_then(|v| v.as_str())
                .ok_or("table_name required")?;
            let filters: Vec<Filter> = args
                .get("filters")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let count = delete_rows(engine.connection(), table, &filters)
                .map_err(|e| e.to_string())?;
            Ok(text_result(format!(
                "{} row(s) deleted from '{}'",
                count, table
            )))
        }

        "get_schema" => {
            let schema = engine.get_schema().map_err(|e| e.to_string())?;
            Ok(text_result(serde_json::to_string_pretty(&schema).unwrap()))
        }

        "get_db_info" => {
            let schema = engine.get_schema().map_err(|e| e.to_string())?;
            let mut total_rows: u64 = 0;
            for t in &schema.tables {
                total_rows += engine.count_rows(&t.name).unwrap_or(0);
            }
            let info = json!({
                "tables": schema.tables.len(),
                "relations": schema.relations.len(),
                "total_rows": total_rows,
                "schema_version": schema.version,
            });
            Ok(text_result(serde_json::to_string_pretty(&info).unwrap()))
        }

        "create_relation" => {
            let rel = RelationDefinition {
                from_table: args
                    .get("from_table")
                    .and_then(|v| v.as_str())
                    .ok_or("from_table required")?
                    .to_string(),
                from_column: args
                    .get("from_column")
                    .and_then(|v| v.as_str())
                    .ok_or("from_column required")?
                    .to_string(),
                to_table: args
                    .get("to_table")
                    .and_then(|v| v.as_str())
                    .ok_or("to_table required")?
                    .to_string(),
                to_column: args
                    .get("to_column")
                    .and_then(|v| v.as_str())
                    .ok_or("to_column required")?
                    .to_string(),
                relation_type: serde_json::from_str(&format!(
                    "\"{}\"",
                    args.get("relation_type")
                        .and_then(|v| v.as_str())
                        .ok_or("relation_type required")?
                ))
                .map_err(|e| format!("Invalid relation_type: {}", e))?,
                cascade: CascadeRules {
                    on_delete: args
                        .get("on_delete")
                        .and_then(|v| v.as_str())
                        .and_then(|s| serde_json::from_str(&format!("\"{}\"", s)).ok())
                        .unwrap_or_default(),
                    on_update: args
                        .get("on_update")
                        .and_then(|v| v.as_str())
                        .and_then(|s| serde_json::from_str(&format!("\"{}\"", s)).ok())
                        .unwrap_or_default(),
                },
            };
            let version = engine
                .create_relation(&rel)
                .map_err(|e| e.to_string())?;
            Ok(text_result(format!(
                "Relation created: {}.{} -> {}.{} (schema version {})",
                rel.from_table, rel.from_column, rel.to_table, rel.to_column, version
            )))
        }

        "count_rows" => {
            let table = args
                .get("table_name")
                .and_then(|v| v.as_str())
                .ok_or("table_name required")?;
            let count = engine.count_rows(table).map_err(|e| e.to_string())?;
            Ok(text_result(format!("{}", count)))
        }

        // list_other_apps_schemas is handled separately in the async path above
        _ => Err(format!("Unknown tool: {}", tool)),
    }
}

/// Handle the `list_other_apps_schemas` tool call by sending a request to the
/// registry via the WebSocket and waiting for the response.
async fn handle_list_other_apps_schemas(
    outbound_tx: Option<&mpsc::Sender<AgentMessage>>,
    schema_signals: Option<&SchemaQuerySignals>,
) -> Result<Value, String> {
    let text_result = |text: String| -> Value {
        json!({ "content": [{ "type": "text", "text": text }] })
    };

    let outbound_tx = outbound_tx
        .ok_or_else(|| "Registry connection not available (running in standalone MCP mode)".to_string())?;
    let schema_signals = schema_signals
        .ok_or_else(|| "Schema signals not available".to_string())?;

    let request_id = uuid::Uuid::new_v4().to_string();

    // Register a oneshot channel to receive the response
    let (tx, rx) = oneshot::channel();
    {
        let mut signals = schema_signals.write().await;
        signals.insert(request_id.clone(), tx);
    }

    // Send the request to the registry
    outbound_tx
        .send(AgentMessage::GetDataverseSchemas {
            request_id: request_id.clone(),
        })
        .await
        .map_err(|_| "Failed to send request to registry (connection closed)".to_string())?;

    // Wait for the response with a 10s timeout
    match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(schemas)) => {
            let json_output = serde_json::to_string_pretty(&schemas)
                .map_err(|e| format!("Failed to serialize schemas: {}", e))?;
            Ok(text_result(json_output))
        }
        Ok(Err(_)) => {
            // Oneshot sender was dropped (e.g., connection lost)
            Err("Registry connection lost while waiting for schemas".to_string())
        }
        Err(_) => {
            // Timeout — clean up the signal
            let mut signals = schema_signals.write().await;
            signals.remove(&request_id);
            Err("Timeout waiting for schemas from registry (10s)".to_string())
        }
    }
}

// ── Deploy tools (Development environment only) ──────────────

fn get_deploy_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "deploy",
            "description": "Deploy a compiled Rust binary to the linked production container. Copies the binary to /opt/app/app on prod, creates the app.service systemd unit if needed, and (re)starts the service. This tool does NOT build — run `cargo build --release` first, then pass the binary path. The binary manages its own configuration (e.g. read from /opt/app/config.toml or environment variables). The deploy is synchronous and blocks until the service is restarted.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "binary_path": {
                        "type": "string",
                        "description": "Absolute path to the compiled Rust binary (e.g. /root/workspace/target/release/my-app)"
                    }
                },
                "required": ["binary_path"]
            }
        }),
        json!({
            "name": "prod_status",
            "description": "Check the status of the linked production container's app.service and deployed binary. Returns whether the service is active, its uptime, and metadata about the binary at /opt/app/app (size, modification date).",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "prod_logs",
            "description": "Get recent logs from the linked production container's app.service (journalctl output).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "lines": {
                        "type": "integer",
                        "description": "Number of log lines to retrieve (default: 50)",
                        "default": 50
                    }
                }
            }
        }),
        json!({
            "name": "prod_exec",
            "description": "Execute a shell command on the linked production container. Useful for creating directories, checking files, installing packages, inspecting the prod environment, etc. The command runs as root inside the prod container.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to execute (e.g. 'ls -la /opt/app/', 'mkdir -p /opt/app/data')"
                    }
                },
                "required": ["command"]
            }
        }),
        json!({
            "name": "prod_push",
            "description": "Copy a local file or directory to the linked production container. For directories, the contents are archived and extracted at the destination. Use this to push config files (.env), static assets, database files, or any other files needed on prod.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "local_path": {
                        "type": "string",
                        "description": "Absolute path to the local file or directory to copy (e.g. /root/workspace/.env, /root/workspace/frontend/dist)"
                    },
                    "remote_path": {
                        "type": "string",
                        "description": "Absolute destination path on the prod container (e.g. /opt/app/.env, /opt/app/dist)"
                    }
                },
                "required": ["local_path", "remote_path"]
            }
        }),
    ]
}

/// Generate the `.mcp.json` content with all tools listed in `autoApprove`.
/// When `is_dev` is true, includes the deploy MCP server.
pub fn generate_mcp_json(is_dev: bool) -> String {
    let dataverse_tools: Vec<String> = get_tool_definitions()
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect();

    let mut servers = serde_json::Map::new();
    servers.insert(
        "dataverse".to_string(),
        json!({
            "command": "/usr/local/bin/hr-agent",
            "args": ["mcp"],
            "autoApprove": dataverse_tools
        }),
    );

    if is_dev {
        let deploy_tools: Vec<String> = get_deploy_tool_definitions()
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect();
        servers.insert(
            "deploy".to_string(),
            json!({
                "command": "/usr/local/bin/hr-agent",
                "args": ["mcp-deploy"],
                "autoApprove": deploy_tools
            }),
        );
    }

    serde_json::to_string_pretty(&json!({ "mcpServers": servers })).unwrap()
}

async fn handle_deploy_tool_call(
    deploy_ctx: Option<&DeployContext>,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    let text_result = |text: String| -> Value {
        json!({ "content": [{ "type": "text", "text": text }] })
    };

    let ctx = deploy_ctx
        .ok_or_else(|| "Deploy tools not available (not a development environment or not connected)".to_string())?;

    if ctx.environment != Environment::Development {
        return Err("Deploy tools are only available in development environments".to_string());
    }

    match tool {
        "deploy" => {
            let binary_path = args
                .get("binary_path")
                .and_then(|v| v.as_str())
                .ok_or("binary_path required")?;

            // Validate binary exists
            let metadata = tokio::fs::metadata(binary_path)
                .await
                .map_err(|e| format!("Cannot access binary at '{}': {}", binary_path, e))?;
            let binary_size = metadata.len();
            if binary_size == 0 {
                return Err("Binary file is empty".to_string());
            }

            info!("Deploying binary: {} ({} bytes)", binary_path, binary_size);

            // Read the binary
            let binary_data = tokio::fs::read(binary_path)
                .await
                .map_err(|e| format!("Failed to read binary: {e}"))?;

            // POST to deploy endpoint as raw binary
            let url = format!("{}/api/applications/{}/deploy", ctx.api_base_url, ctx.app_id);
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

            let resp = client
                .post(&url)
                .header("Content-Type", "application/octet-stream")
                .body(binary_data)
                .send()
                .await
                .map_err(|e| format!("Failed to send deploy request: {e}"))?;

            let status = resp.status();
            let body: Value = resp.json().await
                .map_err(|e| format!("Failed to parse deploy response: {e}"))?;

            if status.is_success() && body.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("Deploy completed");
                Ok(text_result(format!(
                    "Deploy successful!\n\nBinary: {} ({} bytes)\n{}",
                    binary_path, binary_size, message
                )))
            } else {
                let error = body.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error");
                Ok(text_result(format!("Deploy failed: {}", error)))
            }
        }

        "prod_status" => {
            let url = format!("{}/api/applications/{}/prod/status", ctx.api_base_url, ctx.app_id);
            let client = reqwest::Client::new();
            let resp = client.get(&url).send().await
                .map_err(|e| format!("Failed to query prod status: {e}"))?;

            let body: Value = resp.json().await
                .map_err(|e| format!("Failed to parse response: {e}"))?;

            Ok(text_result(serde_json::to_string_pretty(&body).unwrap()))
        }

        "prod_logs" => {
            let lines = args
                .get("lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(50);

            let url = format!(
                "{}/api/applications/{}/prod/logs?lines={}",
                ctx.api_base_url, ctx.app_id, lines
            );
            let client = reqwest::Client::new();
            let resp = client.get(&url).send().await
                .map_err(|e| format!("Failed to query prod logs: {e}"))?;

            let body: Value = resp.json().await
                .map_err(|e| format!("Failed to parse response: {e}"))?;

            if let Some(logs) = body.get("logs").and_then(|v| v.as_str()) {
                Ok(text_result(logs.to_string()))
            } else {
                Ok(text_result(serde_json::to_string_pretty(&body).unwrap()))
            }
        }

        "prod_exec" => {
            let command = args
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or("command required")?;

            let url = format!("{}/api/applications/{}/prod/exec", ctx.api_base_url, ctx.app_id);
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

            let resp = client.post(&url)
                .json(&json!({"command": command}))
                .send()
                .await
                .map_err(|e| format!("Failed to send exec request: {e}"))?;

            let body: Value = resp.json().await
                .map_err(|e| format!("Failed to parse response: {e}"))?;

            if let Some(stdout) = body.get("stdout").and_then(|v| v.as_str()) {
                let stderr = body.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
                let success = body.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
                let mut output = String::new();
                if !success {
                    output.push_str("Command failed!\n\n");
                }
                if !stdout.is_empty() {
                    output.push_str(&format!("STDOUT:\n{}\n", stdout));
                }
                if !stderr.is_empty() {
                    output.push_str(&format!("STDERR:\n{}\n", stderr));
                }
                if output.is_empty() {
                    output = "Command completed (no output)".to_string();
                }
                Ok(text_result(output))
            } else {
                Ok(text_result(serde_json::to_string_pretty(&body).unwrap()))
            }
        }

        "prod_push" => {
            let local_path = args
                .get("local_path")
                .and_then(|v| v.as_str())
                .ok_or("local_path required")?;
            let remote_path = args
                .get("remote_path")
                .and_then(|v| v.as_str())
                .ok_or("remote_path required")?;

            let metadata = tokio::fs::metadata(local_path)
                .await
                .map_err(|e| format!("Cannot access '{}': {}", local_path, e))?;

            let is_dir = metadata.is_dir();

            // Create a tarball of the file/directory
            let tar_path = "/tmp/prod-push-artifact.tar.gz";
            let tar_args = if is_dir {
                vec!["czf", tar_path, "-C", local_path, "."]
            } else {
                // For a single file, tar it from its parent dir with just the filename
                let parent = std::path::Path::new(local_path)
                    .parent()
                    .map(|p| p.to_str().unwrap_or("/"))
                    .unwrap_or("/");
                let filename = std::path::Path::new(local_path)
                    .file_name()
                    .map(|f| f.to_str().unwrap_or("file"))
                    .unwrap_or("file");
                vec!["czf", tar_path, "-C", parent, filename]
            };

            let tar_output = tokio::process::Command::new("tar")
                .args(&tar_args)
                .output()
                .await
                .map_err(|e| format!("Failed to create tarball: {e}"))?;

            if !tar_output.status.success() {
                let stderr = String::from_utf8_lossy(&tar_output.stderr);
                return Err(format!("Failed to create tarball: {stderr}"));
            }

            let archive_data = tokio::fs::read(tar_path)
                .await
                .map_err(|e| format!("Failed to read tarball: {e}"))?;
            let archive_size = archive_data.len();
            let _ = tokio::fs::remove_file(tar_path).await;

            info!("Pushing {} to prod:{} ({} bytes archive, is_dir={})",
                local_path, remote_path, archive_size, is_dir);

            let url = format!("{}/api/applications/{}/prod/push", ctx.api_base_url, ctx.app_id);
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

            let resp = client.post(&url)
                .header("Content-Type", "application/octet-stream")
                .header("X-Remote-Path", remote_path)
                .header("X-Is-Directory", if is_dir { "true" } else { "false" })
                .body(archive_data)
                .send()
                .await
                .map_err(|e| format!("Failed to send push request: {e}"))?;

            let body: Value = resp.json().await
                .map_err(|e| format!("Failed to parse response: {e}"))?;

            if body.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                Ok(text_result(format!(
                    "Pushed {} → prod:{}\nArchive size: {} bytes\nType: {}",
                    local_path, remote_path, archive_size,
                    if is_dir { "directory" } else { "file" }
                )))
            } else {
                let error = body.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error");
                Ok(text_result(format!("Push failed: {}", error)))
            }
        }

        _ => Err(format!("Unknown deploy tool: {}", tool)),
    }
}
