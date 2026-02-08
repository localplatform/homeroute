use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Supported field types for Dataverse columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    Text,
    Number,
    Decimal,
    Boolean,
    DateTime,
    Date,
    Time,
    Email,
    Url,
    Phone,
    Currency,
    Percent,
    Duration,
    Json,
    Uuid,
    AutoIncrement,
    Choice,
    MultiChoice,
    Lookup,
    Formula,
}

impl FieldType {
    /// Returns the SQLite column type for this field type.
    pub fn sqlite_type(&self) -> &'static str {
        match self {
            Self::Text | Self::Email | Self::Url | Self::Phone
            | Self::Json | Self::Uuid | Self::Choice | Self::MultiChoice
            | Self::Formula | Self::Duration => "TEXT",
            Self::Number | Self::AutoIncrement => "INTEGER",
            Self::Decimal | Self::Currency | Self::Percent => "REAL",
            Self::Boolean => "INTEGER", // 0/1
            Self::DateTime => "TEXT",   // ISO 8601
            Self::Date => "TEXT",       // YYYY-MM-DD
            Self::Time => "TEXT",       // HH:MM:SS
            Self::Lookup => "INTEGER",  // foreign key
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDefinition {
    pub name: String,
    pub field_type: FieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub default_value: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Available choices for Choice/MultiChoice fields.
    #[serde(default)]
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableDefinition {
    pub name: String,
    pub slug: String,
    pub columns: Vec<ColumnDefinition>,
    #[serde(default)]
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    OneToMany,
    ManyToMany,
    SelfReferential,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CascadeAction {
    Cascade,
    SetNull,
    Restrict,
}

impl Default for CascadeAction {
    fn default() -> Self {
        Self::Restrict
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadeRules {
    #[serde(default)]
    pub on_delete: CascadeAction,
    #[serde(default)]
    pub on_update: CascadeAction,
}

impl Default for CascadeRules {
    fn default() -> Self {
        Self {
            on_delete: CascadeAction::Restrict,
            on_update: CascadeAction::Cascade,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationDefinition {
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
    pub relation_type: RelationType,
    #[serde(default)]
    pub cascade: CascadeRules,
}

/// Full database schema metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DatabaseSchema {
    pub tables: Vec<TableDefinition>,
    pub relations: Vec<RelationDefinition>,
    pub version: u64,
    pub updated_at: Option<DateTime<Utc>>,
}
