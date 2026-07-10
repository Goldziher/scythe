use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};

use crate::GeneratedCode;

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

/// Inputs for [`CodegenBackend::generate_grouped_query_fn`].
///
/// The grouped query-fn contract carries enough context (analyzed query, both
/// struct names, the flat and split column sets, params, and the grouping key)
/// that passing it positionally trips `clippy::too_many_arguments`. Bundling it
/// into one struct keeps the per-language implementations uniform as backends
/// opt in to grouped codegen.
pub struct GroupedQueryFn<'a> {
    /// Full analyzed query (SQL, name, params, optional_params, deprecated, …).
    pub analyzed: &'a AnalyzedQuery,
    /// Name of the generated parent struct.
    pub parent_struct_name: &'a str,
    /// Name of the generated child struct.
    pub child_struct_name: &'a str,
    /// All resolved columns in flat SELECT order; used for row decoding.
    pub all_columns: &'a [ResolvedColumn],
    /// Resolved columns belonging to the parent struct.
    pub parent_columns: &'a [ResolvedColumn],
    /// Resolved columns belonging to the child struct(s).
    pub child_columns: &'a [ResolvedColumn],
    /// Resolved query parameters.
    pub params: &'a [ResolvedParam],
    /// Grouping key column name in the flat result row
    /// (matches [`scythe_core::analyzer::GroupByConfig::key_column`]).
    pub key_column: &'a str,
}

/// Trait that all codegen backends must implement.
pub trait CodegenBackend: Send + Sync {
    /// The backend's name (e.g. "rust-sqlx", "rust-tokio-postgres").
    fn name(&self) -> &str;

    /// The backend's manifest (type mappings, naming conventions, etc).
    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest;

    /// Generate a row struct for a query result.
    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError>;

    /// Generate a model struct for a table.
    fn generate_model_struct(&self, table_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError>;

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

    /// Generate a file-level header using already generated code as context.
    ///
    /// Backends with conditional imports can inspect generated fragments and avoid
    /// broad engine-level import guesses. The default preserves existing behavior.
    fn file_header_for_results(&self, _generated: &[GeneratedCode]) -> String {
        self.file_header()
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

    /// Generate parent and child structs for a `:grouped` query.
    ///
    /// A `:grouped` query folds flat rows from a normal SQL SELECT into a nested
    /// parent/child structure entirely on the client side — the SQL itself is
    /// unchanged from a regular `:many` query.
    ///
    /// ## Struct layout
    ///
    /// * **Child struct** (`child_struct_name`): contains all `child_columns`.
    ///   Defined first in the output to avoid forward references.
    /// * **Parent struct** (`parent_struct_name`): contains all `parent_columns`
    ///   plus one extra field `children: Vec<child_struct_name>` (or the
    ///   language-native equivalent collection type).
    ///
    /// ## Grouping semantics
    ///
    /// The generated query function fetches flat rows and folds them into an
    /// **order-preserving** list of parent structs, appending each row's child
    /// fields to the matching parent's collection. Equality on `key_column` is
    /// the fold predicate.
    ///
    /// ## Parameters
    ///
    /// * `parent_struct_name` – fully qualified struct/class name for the parent
    ///   (e.g. `"GetUsersWithOrdersRow"`).
    /// * `child_struct_name` – fully qualified struct/class name for the child
    ///   (e.g. `"GetUsersWithOrdersChildRow"`).
    /// * `parent_columns` – resolved columns belonging to the parent table.
    /// * `child_columns` – resolved columns belonging to child table(s).
    /// * `key_column` – SQL column name used as the grouping key; identifies
    ///   the boundary between parent groups (matches [`GroupByConfig::key_column`]).
    ///
    /// ## Return value
    ///
    /// A string containing both struct definitions (child first, parent second),
    /// stored in [`GeneratedCode::row_struct`].
    ///
    /// ## Default implementation
    ///
    /// Returns an error:
    /// *"grouped queries are not yet supported by the '\<name\>' backend"*.
    /// Backends opt in by overriding this method.
    fn generate_grouped_structs(
        &self,
        _parent_struct_name: &str,
        _child_struct_name: &str,
        _parent_columns: &[ResolvedColumn],
        _child_columns: &[ResolvedColumn],
        _key_column: &str,
    ) -> Result<String, ScytheError> {
        Err(ScytheError::new(
            ErrorCode::InternalError,
            format!("grouped queries are not yet supported by the '{}' backend", self.name()),
        ))
    }

    /// Generate the query function for a `:grouped` query.
    ///
    /// The function runs the flat SQL from `analyzed.sql`, decodes each row,
    /// and folds the rows into an **order-preserving** `Vec<parent_struct_name>`
    /// (or the language-native equivalent), grouping by `key_column` and
    /// appending each row's child fields to the matching parent's `children`
    /// collection.
    ///
    /// ## Parameters
    ///
    /// All inputs are bundled in [`GroupedQueryFn`]: the analyzed query, both
    /// generated struct names, the flat and split (parent/child) column sets,
    /// the resolved params, and the grouping key column.
    ///
    /// ## Return value
    ///
    /// A string containing the full query function definition, stored in
    /// [`GeneratedCode::query_fn`].
    ///
    /// ## Default implementation
    ///
    /// Returns an error:
    /// *"grouped queries are not yet supported by the '\<name\>' backend"*.
    /// Backends opt in by overriding this method.
    fn generate_grouped_query_fn(&self, _request: &GroupedQueryFn<'_>) -> Result<String, ScytheError> {
        Err(ScytheError::new(
            ErrorCode::InternalError,
            format!("grouped queries are not yet supported by the '{}' backend", self.name()),
        ))
    }

    /// Apply per-backend configuration options from [[sql.gen]].
    /// Backends override this to handle options like `row_type = "pydantic"`.
    fn apply_options(&mut self, _options: &std::collections::HashMap<String, String>) -> Result<(), ScytheError> {
        Ok(())
    }

    /// Database engines this backend supports.
    /// Defaults to PostgreSQL only. Multi-DB backends override this.
    fn supported_engines(&self) -> &[&str] {
        &["postgresql"]
    }
}
