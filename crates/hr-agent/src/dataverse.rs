//! Local Dataverse database management for the agent.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use tracing::info;

use hr_dataverse::engine::DataverseEngine;
use hr_dataverse::schema::DatabaseSchema;
use hr_registry::protocol::{SchemaColumnInfo, SchemaRelationInfo, SchemaTableInfo};

const DATAVERSE_DIR: &str = "/root/workspace/.dataverse";
const DB_FILENAME: &str = "app.db";

/// Manages the local Dataverse SQLite database.
pub struct LocalDataverse {
    engine: Arc<Mutex<DataverseEngine>>,
    db_path: PathBuf,
}

impl LocalDataverse {
    /// Open or create the local Dataverse database.
    pub fn open() -> Result<Self> {
        let dir = Path::new(DATAVERSE_DIR);
        std::fs::create_dir_all(dir)?;
        let db_path = dir.join(DB_FILENAME);
        let engine = DataverseEngine::open(&db_path)?;
        info!(path = %db_path.display(), "Dataverse database opened");
        Ok(Self {
            engine: Arc::new(Mutex::new(engine)),
            db_path,
        })
    }

    /// Get schema metadata for protocol reporting.
    pub async fn get_schema_metadata(
        &self,
    ) -> Result<(Vec<SchemaTableInfo>, Vec<SchemaRelationInfo>, u64, u64)> {
        let engine = self.engine.lock().await;
        let schema = engine.get_schema()?;

        let mut tables = Vec::new();
        for table in &schema.tables {
            let row_count = engine.count_rows(&table.name).unwrap_or(0);
            tables.push(SchemaTableInfo {
                name: table.name.clone(),
                slug: table.slug.clone(),
                columns: table
                    .columns
                    .iter()
                    .map(|c| SchemaColumnInfo {
                        name: c.name.clone(),
                        field_type: serde_json::to_string(&c.field_type)
                            .unwrap_or_default()
                            .trim_matches('"')
                            .to_string(),
                        required: c.required,
                        unique: c.unique,
                    })
                    .collect(),
                row_count,
            });
        }

        let relations: Vec<SchemaRelationInfo> = schema
            .relations
            .iter()
            .map(|r| SchemaRelationInfo {
                from_table: r.from_table.clone(),
                from_column: r.from_column.clone(),
                to_table: r.to_table.clone(),
                to_column: r.to_column.clone(),
                relation_type: serde_json::to_string(&r.relation_type)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_string(),
            })
            .collect();

        let db_size = DataverseEngine::db_size_bytes(&self.db_path);

        Ok((tables, relations, schema.version, db_size))
    }

    /// Get the database schema.
    pub async fn get_schema(&self) -> Result<DatabaseSchema> {
        let engine = self.engine.lock().await;
        Ok(engine.get_schema()?)
    }

    /// Get the engine (for MCP operations).
    pub fn engine(&self) -> &Arc<Mutex<DataverseEngine>> {
        &self.engine
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}
