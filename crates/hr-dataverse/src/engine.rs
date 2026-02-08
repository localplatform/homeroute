use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection};
use tracing::info;

use crate::migration::{generate_ddl, MigrationOp};
use crate::schema::*;
use crate::validation::*;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Validation error: {0}")]
    Validation(#[from] ValidationError),
    #[error("{0}")]
    Other(String),
}

pub struct DataverseEngine {
    conn: Connection,
}

impl DataverseEngine {
    /// Open or create a Dataverse database.
    pub fn open(path: &Path) -> Result<Self, EngineError> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let engine = Self { conn };
        engine.init_metadata()?;
        Ok(engine)
    }

    /// Open an in-memory database (for testing).
    pub fn open_memory() -> Result<Self, EngineError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let engine = Self { conn };
        engine.init_metadata()?;
        Ok(engine)
    }

    fn init_metadata(&self) -> Result<(), EngineError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _dv_tables (
                name TEXT PRIMARY KEY,
                slug TEXT UNIQUE NOT NULL,
                description TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS _dv_columns (
                table_name TEXT NOT NULL,
                name TEXT NOT NULL,
                field_type TEXT NOT NULL,
                required INTEGER NOT NULL DEFAULT 0,
                is_unique INTEGER NOT NULL DEFAULT 0,
                default_value TEXT,
                description TEXT,
                choices TEXT,
                position INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (table_name, name),
                FOREIGN KEY (table_name) REFERENCES _dv_tables(name) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS _dv_relations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_table TEXT NOT NULL,
                from_column TEXT NOT NULL,
                to_table TEXT NOT NULL,
                to_column TEXT NOT NULL,
                relation_type TEXT NOT NULL,
                on_delete TEXT NOT NULL DEFAULT 'restrict',
                on_update TEXT NOT NULL DEFAULT 'cascade',
                FOREIGN KEY (from_table) REFERENCES _dv_tables(name),
                FOREIGN KEY (to_table) REFERENCES _dv_tables(name)
            );
            CREATE TABLE IF NOT EXISTS _dv_migrations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                description TEXT,
                operations TEXT NOT NULL,
                applied_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS _dv_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            INSERT OR IGNORE INTO _dv_meta (key, value) VALUES ('schema_version', '0');",
        )?;
        Ok(())
    }

    /// Get current schema version.
    pub fn schema_version(&self) -> Result<u64, EngineError> {
        let v: String = self.conn.query_row(
            "SELECT value FROM _dv_meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )?;
        Ok(v.parse().unwrap_or(0))
    }

    fn bump_version_in_tx(&self, tx: &rusqlite::Transaction<'_>) -> Result<u64, EngineError> {
        let v: String = tx.query_row(
            "SELECT value FROM _dv_meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )?;
        let new_v = v.parse::<u64>().unwrap_or(0) + 1;
        tx.execute(
            "UPDATE _dv_meta SET value = ?1 WHERE key = 'schema_version'",
            params![new_v.to_string()],
        )?;
        Ok(new_v)
    }

    /// Create a new table.
    pub fn create_table(&self, table: &TableDefinition) -> Result<u64, EngineError> {
        let schema = self.get_schema()?;
        validate_table_definition(table, &schema)?;

        let tx = self.conn.unchecked_transaction()?;

        // Generate and execute DDL
        let ddl = generate_ddl(&MigrationOp::CreateTable(table.clone()));
        for sql in &ddl {
            tx.execute_batch(sql)?;
        }

        let now = Utc::now().to_rfc3339();
        // Insert metadata
        tx.execute(
            "INSERT INTO _dv_tables (name, slug, description, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![table.name, table.slug, table.description, now, now],
        )?;

        for (i, col) in table.columns.iter().enumerate() {
            let choices_json = if col.choices.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&col.choices).unwrap())
            };
            tx.execute(
                "INSERT INTO _dv_columns (table_name, name, field_type, required, is_unique, default_value, description, choices, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    table.name,
                    col.name,
                    serde_json::to_string(&col.field_type)
                        .unwrap()
                        .trim_matches('"'),
                    col.required as i32,
                    col.unique as i32,
                    col.default_value,
                    col.description,
                    choices_json,
                    i as i32,
                ],
            )?;
        }

        let version = self.bump_version_in_tx(&tx)?;

        // Record migration
        let ops_json =
            serde_json::to_string(&[MigrationOp::CreateTable(table.clone())]).unwrap();
        tx.execute(
            "INSERT INTO _dv_migrations (description, operations, applied_at) VALUES (?1, ?2, ?3)",
            params![format!("Create table '{}'", table.name), ops_json, now],
        )?;

        tx.commit()?;
        info!(table = table.name, "Table created");
        Ok(version)
    }

    /// Add a column to an existing table.
    pub fn add_column(
        &self,
        table_name: &str,
        column: &ColumnDefinition,
    ) -> Result<u64, EngineError> {
        validate_identifier(table_name)?;
        validate_column(column)?;

        // Check table exists
        let exists: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM _dv_tables WHERE name = ?1",
            params![table_name],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(EngineError::Validation(ValidationError::TableNotFound(
                table_name.to_string(),
            )));
        }

        let tx = self.conn.unchecked_transaction()?;

        let op = MigrationOp::AddColumn {
            table: table_name.to_string(),
            column: column.clone(),
        };
        for sql in generate_ddl(&op) {
            tx.execute_batch(&sql)?;
        }

        let now = Utc::now().to_rfc3339();
        let position: i32 = tx.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM _dv_columns WHERE table_name = ?1",
            params![table_name],
            |r| r.get(0),
        )?;

        let choices_json = if column.choices.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&column.choices).unwrap())
        };
        tx.execute(
            "INSERT INTO _dv_columns (table_name, name, field_type, required, is_unique, default_value, description, choices, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                table_name,
                column.name,
                serde_json::to_string(&column.field_type)
                    .unwrap()
                    .trim_matches('"'),
                column.required as i32,
                column.unique as i32,
                column.default_value,
                column.description,
                choices_json,
                position,
            ],
        )?;

        tx.execute(
            "UPDATE _dv_tables SET updated_at = ?1 WHERE name = ?2",
            params![now, table_name],
        )?;

        let version = self.bump_version_in_tx(&tx)?;

        let ops_json = serde_json::to_string(&[&op]).unwrap();
        tx.execute(
            "INSERT INTO _dv_migrations (description, operations, applied_at) VALUES (?1, ?2, ?3)",
            params![
                format!("Add column '{}' to '{}'", column.name, table_name),
                ops_json,
                now
            ],
        )?;

        tx.commit()?;
        info!(table = table_name, column = column.name, "Column added");
        Ok(version)
    }

    /// Drop a table.
    pub fn drop_table(&self, table_name: &str) -> Result<u64, EngineError> {
        validate_identifier(table_name)?;

        let exists: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM _dv_tables WHERE name = ?1",
            params![table_name],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(EngineError::Validation(ValidationError::TableNotFound(
                table_name.to_string(),
            )));
        }

        let tx = self.conn.unchecked_transaction()?;
        let now = Utc::now().to_rfc3339();

        let op = MigrationOp::DropTable { table: table_name.to_string() };
        for sql in generate_ddl(&op) {
            tx.execute_batch(&sql)?;
        }

        // Clean metadata
        tx.execute(
            "DELETE FROM _dv_columns WHERE table_name = ?1",
            params![table_name],
        )?;
        tx.execute(
            "DELETE FROM _dv_relations WHERE from_table = ?1 OR to_table = ?1",
            params![table_name],
        )?;
        tx.execute(
            "DELETE FROM _dv_tables WHERE name = ?1",
            params![table_name],
        )?;

        let version = self.bump_version_in_tx(&tx)?;

        let ops_json = serde_json::to_string(&[&op]).unwrap();
        tx.execute(
            "INSERT INTO _dv_migrations (description, operations, applied_at) VALUES (?1, ?2, ?3)",
            params![format!("Drop table '{}'", table_name), ops_json, now],
        )?;

        tx.commit()?;
        info!(table = table_name, "Table dropped");
        Ok(version)
    }

    /// Remove a column from a table.
    pub fn remove_column(
        &self,
        table_name: &str,
        column_name: &str,
    ) -> Result<u64, EngineError> {
        validate_identifier(table_name)?;
        validate_identifier(column_name)?;

        let tx = self.conn.unchecked_transaction()?;
        let now = Utc::now().to_rfc3339();

        // Verify table and column exist
        let col_exists: bool = tx.query_row(
            "SELECT COUNT(*) > 0 FROM _dv_columns WHERE table_name = ?1 AND name = ?2",
            params![table_name, column_name],
            |r| r.get(0),
        )?;
        if !col_exists {
            return Err(EngineError::Validation(ValidationError::ColumnNotFound(
                column_name.to_string(),
                table_name.to_string(),
            )));
        }

        let op = MigrationOp::RemoveColumn {
            table: table_name.to_string(),
            column: column_name.to_string(),
        };
        for sql in generate_ddl(&op) {
            tx.execute_batch(&sql)?;
        }

        tx.execute(
            "DELETE FROM _dv_columns WHERE table_name = ?1 AND name = ?2",
            params![table_name, column_name],
        )?;
        tx.execute(
            "UPDATE _dv_tables SET updated_at = ?1 WHERE name = ?2",
            params![now, table_name],
        )?;

        let version = self.bump_version_in_tx(&tx)?;

        let ops_json = serde_json::to_string(&[&op]).unwrap();
        tx.execute(
            "INSERT INTO _dv_migrations (description, operations, applied_at) VALUES (?1, ?2, ?3)",
            params![
                format!("Remove column '{}' from '{}'", column_name, table_name),
                ops_json,
                now
            ],
        )?;

        tx.commit()?;
        info!(table = table_name, column = column_name, "Column removed");
        Ok(version)
    }

    /// Create a relation between tables.
    pub fn create_relation(&self, rel: &RelationDefinition) -> Result<u64, EngineError> {
        let schema = self.get_schema()?;
        validate_relation(rel, &schema)?;

        let tx = self.conn.unchecked_transaction()?;
        let now = Utc::now().to_rfc3339();

        let on_delete = serde_json::to_string(&rel.cascade.on_delete)
            .unwrap()
            .trim_matches('"')
            .to_string();
        let on_update = serde_json::to_string(&rel.cascade.on_update)
            .unwrap()
            .trim_matches('"')
            .to_string();
        let rel_type = serde_json::to_string(&rel.relation_type)
            .unwrap()
            .trim_matches('"')
            .to_string();

        tx.execute(
            "INSERT INTO _dv_relations (from_table, from_column, to_table, to_column, relation_type, on_delete, on_update)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                rel.from_table,
                rel.from_column,
                rel.to_table,
                rel.to_column,
                rel_type,
                on_delete,
                on_update
            ],
        )?;

        let version = self.bump_version_in_tx(&tx)?;

        let ops_json =
            serde_json::to_string(&[MigrationOp::CreateRelation { relation: rel.clone() }]).unwrap();
        tx.execute(
            "INSERT INTO _dv_migrations (description, operations, applied_at) VALUES (?1, ?2, ?3)",
            params![
                format!(
                    "Create relation {}.{} -> {}.{}",
                    rel.from_table, rel.from_column, rel.to_table, rel.to_column
                ),
                ops_json,
                now
            ],
        )?;

        tx.commit()?;
        info!(from = %rel.from_table, to = %rel.to_table, "Relation created");
        Ok(version)
    }

    /// Get full database schema from metadata tables.
    pub fn get_schema(&self) -> Result<DatabaseSchema, EngineError> {
        let version = self.schema_version()?;

        let mut tables = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT name, slug, description, created_at, updated_at FROM _dv_tables ORDER BY name",
        )?;
        let table_rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        for row in table_rows {
            let (name, slug, desc, created, updated) = row?;
            let columns = self.get_columns(&name)?;
            tables.push(TableDefinition {
                name,
                slug,
                columns,
                description: desc,
                created_at: chrono::DateTime::parse_from_rfc3339(&created)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: chrono::DateTime::parse_from_rfc3339(&updated)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            });
        }

        let relations = self.get_relations()?;

        Ok(DatabaseSchema {
            tables,
            relations,
            version,
            updated_at: Some(Utc::now()),
        })
    }

    fn get_columns(&self, table_name: &str) -> Result<Vec<ColumnDefinition>, EngineError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, field_type, required, is_unique, default_value, description, choices
             FROM _dv_columns WHERE table_name = ?1 ORDER BY position",
        )?;
        let rows = stmt.query_map(params![table_name], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;

        let mut columns = Vec::new();
        for row in rows {
            let (name, ft_str, required, unique, default_value, desc, choices_json) = row?;
            let field_type: FieldType =
                serde_json::from_str(&format!("\"{}\"", ft_str)).unwrap_or(FieldType::Text);
            let choices: Vec<String> = choices_json
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default();
            columns.push(ColumnDefinition {
                name,
                field_type,
                required,
                unique,
                default_value,
                description: desc,
                choices,
            });
        }
        Ok(columns)
    }

    fn get_relations(&self) -> Result<Vec<RelationDefinition>, EngineError> {
        let mut stmt = self.conn.prepare(
            "SELECT from_table, from_column, to_table, to_column, relation_type, on_delete, on_update
             FROM _dv_relations",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;

        let mut rels = Vec::new();
        for row in rows {
            let (from_t, from_c, to_t, to_c, rel_type_str, on_del_str, on_upd_str) = row?;
            let relation_type: RelationType =
                serde_json::from_str(&format!("\"{}\"", rel_type_str))
                    .unwrap_or(RelationType::OneToMany);
            let on_delete: CascadeAction =
                serde_json::from_str(&format!("\"{}\"", on_del_str)).unwrap_or_default();
            let on_update: CascadeAction =
                serde_json::from_str(&format!("\"{}\"", on_upd_str)).unwrap_or_default();
            rels.push(RelationDefinition {
                from_table: from_t,
                from_column: from_c,
                to_table: to_t,
                to_column: to_c,
                relation_type,
                cascade: CascadeRules {
                    on_delete,
                    on_update,
                },
            });
        }
        Ok(rels)
    }

    /// Get a single table definition.
    pub fn get_table(&self, name: &str) -> Result<Option<TableDefinition>, EngineError> {
        let schema = self.get_schema()?;
        Ok(schema.tables.into_iter().find(|t| t.name == name))
    }

    /// Count rows in a user table.
    pub fn count_rows(&self, table_name: &str) -> Result<u64, EngineError> {
        validate_identifier(table_name)?;
        let sql = format!("SELECT COUNT(*) FROM \"{}\"", table_name);
        let count: u64 = self.conn.query_row(&sql, [], |r| r.get(0))?;
        Ok(count)
    }

    /// Get the database file size in bytes.
    pub fn db_size_bytes(path: &Path) -> u64 {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }

    /// Access the underlying connection (for query module).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}
