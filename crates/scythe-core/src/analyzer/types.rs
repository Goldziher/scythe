use crate::parser::QueryCommand;

// ---------------------------------------------------------------------------
// Public output types
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct AnalyzedQuery {
    pub name: String,
    pub command: QueryCommand,
    pub sql: String,
    pub columns: Vec<AnalyzedColumn>,
    pub params: Vec<AnalyzedParam>,
    pub deprecated: Option<String>,
    /// If this is a SELECT * from a single table, the table name
    pub source_table: Option<String>,
    /// Composite type definitions needed by this query
    pub composites: Vec<CompositeInfo>,
    /// Enum type definitions needed by this query
    pub enums: Vec<EnumInfo>,
    /// Parameter names marked @optional — triggers SQL rewriting in codegen
    pub optional_params: Vec<String>,
    /// Grouping configuration for :grouped queries
    pub group_by: Option<GroupByConfig>,
}

#[derive(Debug, Clone)]
pub struct GroupByConfig {
    /// The table (or alias) used as the grouping parent, e.g. "users"
    pub table: String,
    /// The key column within the parent table, e.g. "id"
    pub key_column: String,
    /// Columns belonging to the parent table
    pub parent_columns: Vec<AnalyzedColumn>,
    /// Columns belonging to child table(s)
    pub child_columns: Vec<AnalyzedColumn>,
}

#[derive(Debug, Clone)]
pub struct CompositeInfo {
    pub sql_name: String,
    pub fields: Vec<CompositeFieldInfo>,
}

#[derive(Debug, Clone)]
pub struct CompositeFieldInfo {
    pub name: String,
    pub neutral_type: String,
}

#[derive(Debug, Clone)]
pub struct EnumInfo {
    pub sql_name: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AnalyzedColumn {
    pub name: String,
    pub neutral_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone)]
pub struct AnalyzedParam {
    pub name: String,
    pub neutral_type: String,
    pub nullable: bool,
    pub position: i64,
}

// ---------------------------------------------------------------------------
// Internal scope types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(super) struct ScopeSource {
    pub(super) alias: String,
    pub(super) table_name: String,
    pub(super) columns: Vec<ScopeColumn>,
    pub(super) nullable_from_join: bool,
}

#[derive(Debug, Clone)]
pub(super) struct ScopeColumn {
    pub(super) name: String,
    pub(super) neutral_type: String,
    pub(super) base_nullable: bool,
}

#[derive(Debug, Clone)]
pub(super) struct Scope {
    pub(super) sources: Vec<ScopeSource>,
}

/// Tracked parameter info during analysis
#[derive(Debug, Clone)]
pub(super) struct ParamInfo {
    pub(super) position: i64,
    pub(super) name: Option<String>,
    pub(super) neutral_type: Option<String>,
    pub(super) nullable: bool,
}

/// Result of inferring an expression's type
#[derive(Debug, Clone)]
pub(super) struct TypeInfo {
    pub(super) neutral_type: String,
    pub(super) nullable: bool,
}

impl TypeInfo {
    pub(super) fn new(neutral_type: impl Into<String>, nullable: bool) -> Self {
        Self {
            neutral_type: neutral_type.into(),
            nullable,
        }
    }
    pub(super) fn unknown() -> Self {
        Self::new("unknown", true)
    }
}

// ---------------------------------------------------------------------------
// Analyzer context
// ---------------------------------------------------------------------------

use ahash::AHashMap;

use crate::catalog::Catalog;

pub(super) struct Analyzer<'a> {
    pub(super) catalog: &'a Catalog,
    pub(super) params: Vec<ParamInfo>,
    /// CTE name -> columns
    pub(super) ctes: AHashMap<String, Vec<ScopeColumn>>,
    /// Collected type errors during analysis
    pub(super) type_errors: Vec<String>,
    /// Auto-incrementing counter for MySQL `?` positional placeholders
    pub(super) positional_param_counter: i64,
}
