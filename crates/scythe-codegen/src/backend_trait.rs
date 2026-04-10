use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::ScytheError;

/// Information needed to generate an RBS type signature file.
#[derive(Debug, Clone)]
pub struct RbsGenerationContext {
    pub queries: Vec<RbsQueryInfo>,
    pub enums: Vec<RbsEnumInfo>,
}

/// Per-query info for RBS generation.
#[derive(Debug, Clone)]
pub struct RbsQueryInfo {
    pub func_name: String,
    pub struct_name: Option<String>,
    pub columns: Vec<ResolvedColumn>,
    pub params: Vec<ResolvedParam>,
    pub command: scythe_core::parser::QueryCommand,
}

/// Per-enum info for RBS generation.
#[derive(Debug, Clone)]
pub struct RbsEnumInfo {
    pub type_name: String,
    pub values: Vec<String>,
}

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

    /// The backend's manifest (type mappings, naming conventions, etc).
    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest;

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

    /// Generate a file-level header (imports, docstring, etc).
    /// Returns an empty string by default; backends may override.
    fn file_header(&self) -> String {
        String::new()
    }

    /// Generate a file-level footer (closing braces, etc).
    /// Returns an empty string by default; backends may override.
    fn file_footer(&self) -> String {
        String::new()
    }

    /// Generate a class header that wraps query functions only.
    /// When non-empty, the assembly will emit all type definitions (enums,
    /// row structs, model structs) first, then this class header, then all
    /// query functions, then the file footer.
    /// Returns an empty string by default (no class wrapper).
    fn query_class_header(&self) -> String {
        String::new()
    }

    /// Generate code that should be emitted after the file footer.
    /// This is useful for backends that need top-level code after a class wrapper.
    /// For example, C# extension methods must be top-level, not nested.
    /// Returns an empty string by default.
    fn post_footer(&self) -> String {
        String::new()
    }

    /// Generate an RBS type signature file for Ruby backends.
    /// Returns `None` by default; Ruby backends override this.
    fn generate_rbs_file(&self, _context: &RbsGenerationContext) -> Option<String> {
        None
    }

    /// Apply per-backend configuration options from [[sql.gen]].
    /// Backends override this to handle options like `row_type = "pydantic"`.
    fn apply_options(
        &mut self,
        _options: &std::collections::HashMap<String, String>,
    ) -> Result<(), ScytheError> {
        Ok(())
    }

    /// Database engines this backend supports.
    /// Defaults to PostgreSQL only. Multi-DB backends override this.
    fn supported_engines(&self) -> &[&str] {
        &["postgresql"]
    }
}
