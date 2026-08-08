use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case, to_snake_case,
};
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};
use crate::backends::typescript_common::parse_bool_option;
use crate::singularize;

/// Default embedded manifest TOML for rust-sqlx, used as fallback.
const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/rust-sqlx.toml");
const DEFAULT_MANIFEST_MYSQL: &str = include_str!("../../manifests/rust-sqlx.mysql.toml");
const DEFAULT_MANIFEST_MARIADB: &str = include_str!("../../manifests/rust-sqlx.mariadb.toml");
const DEFAULT_MANIFEST_REDSHIFT: &str = include_str!("../../manifests/rust-sqlx.redshift.toml");
const DEFAULT_MANIFEST_SQLITE: &str = include_str!("../../manifests/rust-sqlx.sqlite.toml");

/// SqlxBackend generates Rust code targeting the sqlx crate.
pub struct SqlxBackend {
    manifest: BackendManifest,
    engine: String,
    /// When true, only emit struct/enum definitions (no query functions).
    /// This avoids the `sqlx::query_as!()` macro which requires `DATABASE_URL` at compile time.
    structs_only: bool,
    /// Whether this engine's manifest declares the `json_nested` container
    /// and its server actually has `json_agg`. See
    /// [`crate::backends::engine_supports_nested_aggregates`].
    nested_aggregates: bool,
}

impl SqlxBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "postgresql" | "postgres" | "pg" | "mysql" | "mariadb" | "sqlite" | "sqlite3" | "redshift" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("unsupported engine '{}' for rust-sqlx backend", engine),
                ));
            }
        }
        let manifest = match engine {
            "mysql" => super::parse_manifest(DEFAULT_MANIFEST_MYSQL)?,
            "mariadb" => super::parse_manifest(DEFAULT_MANIFEST_MARIADB)?,
            "redshift" => super::parse_manifest(DEFAULT_MANIFEST_REDSHIFT)?,
            "sqlite" | "sqlite3" => super::parse_manifest(DEFAULT_MANIFEST_SQLITE)?,
            _ => super::parse_manifest(DEFAULT_MANIFEST_TOML)?,
        };
        Ok(Self {
            manifest,
            engine: engine.to_string(),
            structs_only: false,
            nested_aggregates: super::engine_supports_nested_aggregates(engine),
        })
    }
}

impl SqlxBackend {
    /// Return true if this engine uses inline ENUMs (not named custom types).
    ///
    /// MySQL, MariaDB, and SQLite represent ENUMs as plain strings at the wire
    /// level. sqlx's `#[derive(sqlx::Type)]` generates `type_info()` returning
    /// `MySqlTypeInfo::__enum()` (ColumnType::String + ENUM flag), but the
    /// server sends `ColumnType::Enum`. The PartialEq check in MySqlTypeInfo
    /// fails because the r#type fields differ, producing a runtime
    /// "mismatched types" ColumnDecode error.
    ///
    /// For these engines, row struct fields must use `String` (or `Option<String>`)
    /// instead of the generated Rust enum type.
    fn uses_inline_enums(&self) -> bool {
        matches!(self.engine.as_str(), "mysql" | "mariadb" | "sqlite" | "sqlite3")
    }

    /// Resolve the field type for a row struct column.
    ///
    /// For engines that use inline ENUMs, enum-typed columns are mapped to
    /// `String` / `Option<String>` because sqlx cannot type-check them against
    /// the generated Rust enum at runtime (see `uses_inline_enums`).
    fn row_field_type<'a>(&self, col: &'a ResolvedColumn) -> &'a str {
        if self.uses_inline_enums() && col.neutral_type.starts_with("enum::") {
            if col.nullable { "Option<String>" } else { "String" }
        } else {
            &col.full_type
        }
    }

    /// Return the sqlx pool type for the configured engine.
    fn pool_type(&self) -> &str {
        match self.engine.as_str() {
            "mysql" | "mariadb" => "sqlx::MySqlPool",
            "sqlite" | "sqlite3" => "sqlx::SqlitePool",
            _ => "sqlx::PgPool",
        }
    }

    /// Return the sqlx query-result type for the configured engine.
    fn query_result_type(&self) -> &str {
        match self.engine.as_str() {
            "mysql" | "mariadb" => "sqlx::mysql::MySqlQueryResult",
            "sqlite" | "sqlite3" => "sqlx::sqlite::SqliteQueryResult",
            _ => "sqlx::postgres::PgQueryResult",
        }
    }
}

impl CodegenBackend for SqlxBackend {
    fn name(&self) -> &str {
        "rust-sqlx"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "mysql", "mariadb", "sqlite", "redshift"]
    }

    fn file_header(&self) -> String {
        "// Auto-generated by scythe. Do not edit.\n#![allow(dead_code, unused_imports, clippy::needless_question_mark, clippy::redundant_closure)]"
            .to_string()
    }

    fn apply_options(&mut self, options: &std::collections::HashMap<String, String>) -> Result<(), ScytheError> {
        if let Some(value) = options.get("structs_only") {
            self.structs_only = parse_bool_option("structs_only", value)?;
        }
        Ok(())
    }

    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        let mut out = String::new();

        let _ = writeln!(out, "#[derive(Debug, Clone, sqlx::FromRow)]");
        let _ = writeln!(out, "pub struct {} {{", struct_name);

        for col in columns {
            let field_type = self.row_field_type(col);
            let _ = writeln!(out, "    pub {}: {},", col.field_name, field_type);
        }

        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_model_struct(&self, table_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let singular = singularize(table_name);
        let struct_name = to_pascal_case(&singular).into_owned();
        let mut out = String::new();

        let _ = writeln!(out, "#[derive(Debug, Clone, sqlx::FromRow)]");
        let _ = writeln!(out, "pub struct {} {{", struct_name);

        for col in columns {
            let field_type = self.row_field_type(col);
            let _ = writeln!(out, "    pub {}: {},", col.field_name, field_type);
        }

        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        _columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        if self.structs_only {
            return Ok(String::new());
        }

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        if let Some(ref msg) = analyzed.deprecated {
            let _ = writeln!(out, "#[deprecated(note = \"{}\")]", msg);
        }

        let pool_type = self.pool_type();
        let mut param_parts: Vec<String> = vec![format!("pool: &{}", pool_type)];
        for param in params {
            param_parts.push(format!("{}: {}", param.field_name, param.borrowed_type));
        }

        let sql_raw = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let sql = rewrite_sql_for_enums(&sql_raw, &analyzed.columns, &self.manifest);

        let bind_params: String = analyzed
            .params
            .iter()
            .map(|p| {
                let param_name = to_snake_case(&p.name);
                if p.neutral_type.starts_with("enum::") {
                    let enum_name = p.neutral_type.strip_prefix("enum::").unwrap();
                    let rust_type = enum_type_name(enum_name, &self.manifest.naming);
                    format!(", {} as &{}", param_name, rust_type)
                } else {
                    format!(", {}", param_name)
                }
            })
            .collect();

        if matches!(analyzed.command, QueryCommand::Batch) {
            let batch_fn_name = format!("{}_batch", func_name);

            if params.len() > 1 {
                let params_struct_name = format!("{}BatchParams", struct_name);
                let _ = writeln!(out, "#[derive(Debug, Clone)]");
                let _ = writeln!(out, "pub struct {} {{", params_struct_name);
                for param in params {
                    let _ = writeln!(out, "    pub {}: {},", param.field_name, param.full_type);
                }
                let _ = writeln!(out, "}}");
                let _ = writeln!(out);

                let _ = writeln!(
                    out,
                    "pub async fn {}(pool: &{}, items: &[{}]) -> Result<(), sqlx::Error> {{",
                    batch_fn_name, pool_type, params_struct_name
                );
                let _ = writeln!(out, "    let mut tx = pool.begin().await?;");
                let _ = writeln!(out, "    for item in items {{");

                let struct_bind_params: String = params
                    .iter()
                    .map(|p| {
                        if p.neutral_type.starts_with("enum::") {
                            let enum_name = p.neutral_type.strip_prefix("enum::").unwrap();
                            let rust_type = enum_type_name(enum_name, &self.manifest.naming);
                            format!(", item.{} as &{}", p.field_name, rust_type)
                        } else {
                            format!(", item.{}", p.field_name)
                        }
                    })
                    .collect();

                let _ = writeln!(out, "        sqlx::query!(\"{}\"{})", sql, struct_bind_params);
                let _ = writeln!(out, "            .execute(&mut *tx)");
                let _ = writeln!(out, "            .await?;");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "    tx.commit().await?;");
                let _ = writeln!(out, "    Ok(())");
            } else if params.len() == 1 {
                let param = &params[0];
                let _ = writeln!(
                    out,
                    "pub async fn {}(pool: &{}, items: &[{}]) -> Result<(), sqlx::Error> {{",
                    batch_fn_name, pool_type, param.full_type
                );
                let _ = writeln!(out, "    let mut tx = pool.begin().await?;");
                let _ = writeln!(out, "    for item in items {{");
                let _ = writeln!(out, "        sqlx::query!(\"{}\", item)", sql);
                let _ = writeln!(out, "            .execute(&mut *tx)");
                let _ = writeln!(out, "            .await?;");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "    tx.commit().await?;");
                let _ = writeln!(out, "    Ok(())");
            } else {
                let _ = writeln!(
                    out,
                    "pub async fn {}(pool: &{}, count: usize) -> Result<(), sqlx::Error> {{",
                    batch_fn_name, pool_type
                );
                let _ = writeln!(out, "    let mut tx = pool.begin().await?;");
                let _ = writeln!(out, "    for _ in 0..count {{");
                let _ = writeln!(out, "        sqlx::query!(\"{}\")", sql);
                let _ = writeln!(out, "            .execute(&mut *tx)");
                let _ = writeln!(out, "            .await?;");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "    tx.commit().await?;");
                let _ = writeln!(out, "    Ok(())");
            }

            let _ = write!(out, "}}");
            return Ok(out);
        }

        let return_type = match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => struct_name.to_string(),
            QueryCommand::Many => format!("Vec<{}>", struct_name),
            QueryCommand::Exec => "()".to_string(),
            QueryCommand::ExecResult => self.query_result_type().to_string(),
            QueryCommand::ExecRows => "u64".to_string(),
            QueryCommand::Batch => unreachable!(),
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
        };

        let _ = writeln!(
            out,
            "pub async fn {}({}) -> Result<{}, sqlx::Error> {{",
            func_name,
            param_parts.join(", "),
            return_type
        );

        let has_row_struct = matches!(analyzed.command, QueryCommand::One | QueryCommand::Many);

        let is_exec_rows = matches!(analyzed.command, QueryCommand::ExecRows);

        if is_exec_rows {
            if has_row_struct && !analyzed.columns.is_empty() {
                let _ = write!(
                    out,
                    "    let result = sqlx::query_as!({}, \"{}\"{})",
                    struct_name, sql, bind_params
                );
            } else {
                let _ = write!(out, "    let result = sqlx::query!(\"{}\"{})", sql, bind_params);
            }
        } else if has_row_struct && !analyzed.columns.is_empty() {
            let _ = write!(out, "    sqlx::query_as!({}, \"{}\"{})", struct_name, sql, bind_params);
        } else {
            let _ = write!(out, "    sqlx::query!(\"{}\"{})", sql, bind_params);
        }

        let _ = writeln!(out);

        let fetch_method = match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => ".fetch_one(pool)",
            QueryCommand::Many => ".fetch_all(pool)",
            QueryCommand::Exec => ".execute(pool)",
            QueryCommand::ExecResult => ".execute(pool)",
            QueryCommand::ExecRows => ".execute(pool)",
            QueryCommand::Batch => unreachable!(),
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
        };

        let _ = write!(out, "        {}", fetch_method);
        let _ = writeln!(out);

        match &analyzed.command {
            QueryCommand::Exec => {
                let _ = writeln!(out, "        .await?;");
                let _ = writeln!(out, "    Ok(())");
            }
            QueryCommand::ExecRows => {
                let _ = writeln!(out, "        .await?;");
                let _ = writeln!(out, "    Ok(result.rows_affected())");
            }
            _ => {
                let _ = writeln!(out, "        .await");
            }
        }

        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let mut out = String::with_capacity(256);
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);

        let _ = writeln!(out, "#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]");
        match self.engine.as_str() {
            "mysql" | "mariadb" | "sqlite" | "sqlite3" => {
                let _ = writeln!(out, "#[sqlx(rename_all = \"snake_case\")]");
            }
            _ => {
                let _ = writeln!(
                    out,
                    "#[sqlx(type_name = \"{}\", rename_all = \"snake_case\")]",
                    enum_info.sql_name
                );
            }
        }
        let _ = writeln!(out, "pub enum {type_name} {{");

        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "    {variant},");
        }

        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_grouped_structs(
        &self,
        parent_struct_name: &str,
        child_struct_name: &str,
        parent_columns: &[crate::backend_trait::ResolvedColumn],
        child_columns: &[crate::backend_trait::ResolvedColumn],
        _key_column: &str,
    ) -> Result<String, ScytheError> {
        let mut out = String::new();

        let _ = writeln!(out, "#[derive(Debug, Clone, sqlx::FromRow)]");
        let _ = writeln!(out, "pub struct {child_struct_name} {{");
        for col in child_columns {
            let field_type = self.row_field_type(col);
            let _ = writeln!(out, "    pub {}: {field_type},", col.field_name);
        }
        let _ = writeln!(out, "}}");

        let _ = writeln!(out);

        let _ = writeln!(out, "#[derive(Debug, Clone)]");
        let _ = writeln!(out, "pub struct {parent_struct_name} {{");
        for col in parent_columns {
            let field_type = self.row_field_type(col);
            let _ = writeln!(out, "    pub {}: {field_type},", col.field_name);
        }
        let _ = writeln!(out, "    pub children: Vec<{child_struct_name}>,");
        let _ = write!(out, "}}");

        Ok(out)
    }

    fn generate_grouped_query_fn(
        &self,
        request: &crate::backend_trait::GroupedQueryFn<'_>,
    ) -> Result<String, ScytheError> {
        let analyzed = request.analyzed;
        let parent_struct_name = request.parent_struct_name;
        let child_struct_name = request.child_struct_name;
        let parent_columns = request.parent_columns;
        let child_columns = request.child_columns;
        let params = request.params;
        let key_column = request.key_column;

        if self.structs_only {
            return Ok(String::new());
        }

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let pool_type = self.pool_type();
        let key_field = to_snake_case(key_column);
        let mut out = String::new();

        if let Some(ref msg) = analyzed.deprecated {
            let _ = writeln!(out, "#[deprecated(note = \"{msg}\")]");
        }

        let mut param_parts: Vec<String> = vec![format!("pool: &{pool_type}")];
        for param in params {
            param_parts.push(format!("{}: {}", param.field_name, param.borrowed_type));
        }

        let sql_raw = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let sql = rewrite_sql_for_enums(&sql_raw, &analyzed.columns, &self.manifest);

        let bind_params: String = analyzed
            .params
            .iter()
            .map(|p| {
                let pname = to_snake_case(&p.name);
                if p.neutral_type.starts_with("enum::") {
                    let enum_name = p.neutral_type.strip_prefix("enum::").unwrap();
                    let rust_type = enum_type_name(enum_name, &self.manifest.naming);
                    format!(", {pname} as &{rust_type}")
                } else {
                    format!(", {pname}")
                }
            })
            .collect();

        let _ = writeln!(
            out,
            "pub async fn {func_name}({}) -> Result<Vec<{parent_struct_name}>, sqlx::Error> {{",
            param_parts.join(", ")
        );

        let _ = writeln!(out, "    let flat_rows = sqlx::query!(\"{sql}\"{bind_params})");
        let _ = writeln!(out, "        .fetch_all(pool)");
        let _ = writeln!(out, "        .await?;");

        let _ = writeln!(out, "    let mut result: Vec<{parent_struct_name}> = Vec::new();");
        let _ = writeln!(out, "    for row in flat_rows {{");

        let _ = writeln!(out, "        let key = row.{key_field}.clone();");

        let _ = writeln!(out, "        let child = {child_struct_name} {{");
        for col in child_columns {
            let _ = writeln!(out, "            {}: row.{},", col.field_name, col.field_name);
        }
        let _ = writeln!(out, "        }};");

        let _ = writeln!(
            out,
            "        if let Some(parent) = result.iter_mut().rev().find(|p| p.{key_field} == key) {{"
        );
        let _ = writeln!(out, "            parent.children.push(child);");
        let _ = writeln!(out, "        }} else {{");
        let _ = writeln!(out, "            result.push({parent_struct_name} {{");
        for col in parent_columns {
            let _ = writeln!(out, "                {}: row.{},", col.field_name, col.field_name);
        }
        let _ = writeln!(out, "                children: vec![child],");
        let _ = writeln!(out, "            }});");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "    Ok(result)");
        let _ = write!(out, "}}");

        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        use scythe_backend::types::resolve_type;

        let struct_name = to_pascal_case(&composite.sql_name).into_owned();
        let mut out = String::new();

        let _ = writeln!(out, "#[derive(Debug, Clone, sqlx::Type)]");
        let _ = writeln!(out, "#[sqlx(type_name = \"{}\")]", composite.sql_name);
        let _ = writeln!(out, "pub struct {} {{", struct_name);
        for field in &composite.fields {
            let rust_type = resolve_type(&field.neutral_type, &self.manifest, false)
                .map(|t| t.into_owned())
                .map_err(|e| {
                    ScytheError::new(ErrorCode::InternalError, format!("composite field type error: {}", e))
                })?;
            let _ = writeln!(out, "    pub {}: {},", to_snake_case(&field.name), rust_type);
        }
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_nested_struct_def(
        &self,
        nested: &scythe_core::analyzer::NestedStructInfo,
    ) -> Result<Option<String>, ScytheError> {
        if !self.nested_aggregates {
            return Ok(None);
        }

        // Unlike generate_composite_def (always `false` -- CompositeFieldInfo
        // has no per-field nullability), a nested-aggregate field's
        // nullability is real and comes from the source column it was
        // built from.
        Ok(Some(generate_nested_rust_struct(nested, &self.manifest)?))
    }

    fn generate_enum_def_for_nested(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        // The plain form derives sqlx::Type only, which decodes an enum off
        // the *wire*. Inside a json_agg result the value arrives as a JSON
        // string decoded by serde_json instead, so without Deserialize the
        // nested struct's own derive fails to satisfy its bound and the file
        // does not compile. Serialize comes along for the reason spelled out
        // in `generate_nested_rust_struct`.
        let base = self.generate_enum_def(enum_info)?;
        Ok(add_serde_to_enum(&base, enum_info, &self.manifest))
    }

    fn generate_composite_def_for_nested(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let base = self.generate_composite_def(composite)?;
        Ok(add_serde_to_first_derive(&base))
    }
}

/// Render a nested-aggregate struct for the two Rust backends.
///
/// Shared because sqlx and tokio-postgres need byte-for-byte the same
/// declaration: both resolve `json_nested<T>` to a `Json<T>` wrapper whose
/// `FromSql`/`Decode` impl is bounded on `T: serde::Deserialize`, and
/// neither can express the JSON key mapping any other way.
///
/// Both serde traits are derived unconditionally, not just `Deserialize`:
/// - `Deserialize` is what actually decodes the column, and is required
///   regardless of any `serde` backend option, which governs only whether
///   the *row* struct opts in (that one is built by `from_row`/`row.get`
///   and never JSON-decoded, an unrelated question);
/// - `Serialize` because a row struct built with `serde = true` derives
///   `Serialize`, its nested field is `Option<Json<Vec<T>>>`, and
///   `Json<T>: Serialize` is bounded on `T: Serialize`. Deriving only
///   `Deserialize` breaks every user who combines `serde = true` with one
///   `json_agg` column. Deriving both unconditionally costs an impl nobody
///   calls in the `serde = false` case, versus a compile error in the other.
pub(crate) fn generate_nested_rust_struct(
    nested: &scythe_core::analyzer::NestedStructInfo,
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    use scythe_backend::types::resolve_type;

    let struct_name = to_pascal_case(&nested.name).into_owned();
    let mut out = String::new();

    let _ = writeln!(out, "#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]");
    let _ = writeln!(out, "pub struct {} {{", struct_name);
    for field in &nested.fields {
        let rust_type = resolve_type(&field.neutral_type, manifest, field.nullable)
            .map(|t| t.into_owned())
            .map_err(|e| {
                ScytheError::new(
                    ErrorCode::InternalError,
                    format!("nested struct field type error: {}", e),
                )
            })?;
        let field_name = to_snake_case(&field.name);
        // The JSON keys json_agg/row_to_json produce are the raw SQL column
        // names, verbatim -- a quoted "createdAt" column is the key
        // "createdAt". Renaming the Rust field to snake_case without telling
        // serde turns that into `missing field \`created_at\`` at runtime,
        // which no build-time check catches.
        if field_name != field.name {
            let _ = writeln!(out, "    #[serde(rename = \"{}\")]", field.name);
        }
        let _ = writeln!(out, "    pub {}: {},", field_name, rust_type);
    }
    let _ = write!(out, "}}");
    Ok(out)
}

/// Add `serde::Serialize, serde::Deserialize` to the first `#[derive(...)]`
/// line of a generated Rust definition, leaving everything else untouched.
///
/// Rewriting the rendered output rather than parameterizing every
/// enum/composite emitter keeps the non-nested path -- the one all existing
/// output goes through -- literally unchanged.
pub(crate) fn add_serde_to_first_derive(rendered: &str) -> String {
    const SERDE: &str = ", serde::Serialize, serde::Deserialize)]";
    let Some(line_end) = rendered.find('\n') else {
        return rendered.to_string();
    };
    let (first, rest) = rendered.split_at(line_end);
    if !first.starts_with("#[derive(") || !first.ends_with(")]") || first.contains("serde::") {
        return rendered.to_string();
    }
    format!("{}{}{}", &first[..first.len() - 2], SERDE, rest)
}

/// [`add_serde_to_first_derive`] plus a `#[serde(rename = "...")]` on every
/// variant whose generated identifier differs from the SQL label.
///
/// The derive alone is not enough. `#[sqlx(rename_all = "snake_case")]` (or
/// tokio-postgres's `Display`/`FromStr` pair) tells the *driver* how the
/// label is spelled on the wire; serde knows nothing about either, and would
/// look for the PascalCase identifier. A `json_agg` result carries the SQL
/// label verbatim, so `'active'` against a `Active` variant fails with
/// `unknown variant \`active\``. Renaming to the label rather than applying a
/// blanket `rename_all` is exact for any label -- including ones that are not
/// snake_case at all, like `'IN PROGRESS'`.
pub(crate) fn add_serde_to_enum(rendered: &str, enum_info: &EnumInfo, manifest: &BackendManifest) -> String {
    let with_derive = add_serde_to_first_derive(rendered);

    let mut out = String::with_capacity(with_derive.len() + enum_info.values.len() * 32);
    let mut values = enum_info.values.iter();
    let mut in_body = false;
    for line in with_derive.split_inclusive('\n') {
        if !in_body {
            out.push_str(line);
            if line.trim_end().ends_with('{') {
                in_body = true;
            }
            continue;
        }
        if line.trim_end() == "}" {
            // Past the enum body; anything after (Display/FromStr impls)
            // is copied through untouched.
            in_body = false;
            out.push_str(line);
            continue;
        }
        match values.next() {
            Some(value) => {
                let variant = enum_variant_name(value, &manifest.naming);
                if variant != *value {
                    let _ = writeln!(out, "    #[serde(rename = \"{}\")]", value);
                }
                out.push_str(line);
            }
            None => out.push_str(line),
        }
    }
    out
}

/// Rewrite SQL to add enum type annotations for sqlx.
fn rewrite_sql_for_enums(sql: &str, columns: &[AnalyzedColumn], manifest: &BackendManifest) -> String {
    let enum_cols: Vec<(&str, String)> = columns
        .iter()
        .filter_map(|col| {
            if let Some(enum_name) = col.neutral_type.strip_prefix("enum::") {
                let rust_type = enum_type_name(enum_name, &manifest.naming);
                let annotation = if col.nullable {
                    format!("Option<{}>", rust_type)
                } else {
                    rust_type
                };
                Some((col.name.as_str(), annotation))
            } else {
                None
            }
        })
        .collect();

    if enum_cols.is_empty() {
        return sql.to_string();
    }

    let mut result = sql.to_string();
    for (col_name, annotation) in &enum_cols {
        let alias = format!("{} AS \\\"{}: {}\\\"", col_name, col_name, annotation);
        if let Some(from_pos) = result.to_uppercase().find(" FROM ") {
            let select_part = &result[..from_pos];
            let rest = &result[from_pos..];
            let new_select = replace_column_in_select(select_part, col_name, &alias);
            result = format!("{}{}", new_select, rest);
        }
    }
    result
}

fn replace_column_in_select(select: &str, col_name: &str, replacement: &str) -> String {
    let mut result = select.to_string();
    let patterns = [format!(", {}", col_name), format!(" {}", col_name)];
    for pattern in &patterns {
        if let Some(pos) = result.rfind(pattern.as_str()) {
            let after = pos + pattern.len();
            let next_char = result[after..].chars().next();
            if next_char.is_none() || matches!(next_char, Some(' ') | Some(',') | Some('\n')) {
                let prefix = &result[..pos + pattern.len() - col_name.len()];
                let suffix = &result[after..];
                result = format!("{}{}{}", prefix, replacement, suffix);
                break;
            }
        }
    }
    result
}

#[cfg(test)]
mod option_tests {
    use super::*;

    #[test]
    fn structs_only_option_applies_when_true() {
        let mut backend = SqlxBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "structs_only".to_string(),
                "true".to_string(),
            )]))
            .unwrap();
        assert!(backend.structs_only);
    }

    /// An unrecognized value must be reported, not silently treated as
    /// leaving `structs_only` disabled.
    #[test]
    fn structs_only_option_rejects_invalid_value() {
        let mut backend = SqlxBackend::new("postgresql").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "structs_only".to_string(),
            "maybe".to_string(),
        )]));
        assert!(result.is_err(), "expected 'maybe' to be rejected");
    }
}
