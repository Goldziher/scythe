use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::ScytheError;

/// A column with its type resolved to the target language.
#[derive(Debug, Clone)]
pub struct ResolvedColumn {
    pub name: String,
    pub field_name: String,
    pub lang_type: String,
    pub full_type: String,
    pub neutral_type: String,
    pub nullable: bool,
}

/// A parameter with its type resolved to the target language.
#[derive(Debug, Clone)]
pub struct ResolvedParam {
    pub name: String,
    pub field_name: String,
    pub lang_type: String,
    pub full_type: String,
    pub borrowed_type: String,
    pub neutral_type: String,
    pub nullable: bool,
}

/// Trait that all codegen backends must implement.
pub trait CodegenBackend: Send + Sync {
    /// The backend's name (e.g. "rust-sqlx", "rust-tokio-postgres").
    fn name(&self) -> &str;

    /// Generate a row struct for a query result.
    fn generate_row_struct(
        &self,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError>;

    /// Generate a model struct for a table.
    fn generate_model_struct(
        &self,
        table_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError>;

    /// Generate a query function.
    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError>;

    /// Generate an enum definition.
    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError>;

    /// Generate a composite type definition.
    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError>;
}
