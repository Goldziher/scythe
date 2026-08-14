use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    composite_type_name, enum_type_name, enum_variant_name, fn_name, to_pascal_case, to_snake_case,
};
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_options::reject_unknown_options;
use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};
use crate::backends::typescript_common::parse_bool_option;

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
    /// When true, every generated struct/enum derives `serde::Serialize` and
    /// `serde::Deserialize` in addition to its base derives. Set via the
    /// `serde` backend option -- see [`Self::apply_options`].
    serde: bool,
    /// Extra derive macros appended after the base derives (and after serde,
    /// when enabled) on every generated struct/enum. Set via the `derive`
    /// backend option -- see [`Self::apply_options`].
    extra_derives: Vec<String>,
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
                    ErrorCode::InvalidConfig,
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
            serde: false,
            extra_derives: Vec::new(),
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

    /// Build a `#[derive(...)]` line from a fixed `base` list plus the
    /// `serde`/`derive` backend options.
    ///
    /// Every struct/enum this backend emits derives a different fixed base
    /// (`sqlx::FromRow` on row-shaped structs, `sqlx::Type` plus its
    /// `#[sqlx(...)]` attribute on enums and composites, plain `Debug, Clone`
    /// on structs sqlx never decodes a row into) but all four must layer the
    /// same optional serde/extra derives on top, so the shared part lives
    /// here instead of being copied at each call site.
    fn derive_line(&self, base: &[&str]) -> String {
        let mut derives: Vec<String> = base.iter().map(|s| s.to_string()).collect();
        if self.serde {
            derives.push("serde::Serialize".to_string());
            derives.push("serde::Deserialize".to_string());
        }
        // ~keep `base` always includes "Debug" and "Clone" (every call site does; some also
        // add "PartialEq", "Eq"), so a `derive` option that happens to name one of those --
        // plausible when a user thinks they need to ask for `Debug` explicitly, or sets
        // both `serde = true` and lists `serde::Serialize` in `derive` -- would otherwise
        // append a second, identical derive token. Two `impl Debug for Foo` from two
        // identical `#[derive(Debug)]` expansions is E0119 in the generated file, not a
        // harmless no-op.
        for extra in &self.extra_derives {
            if !derives.iter().any(|d| d == extra) {
                derives.push(extra.clone());
            }
        }
        format!("#[derive({})]", derives.join(", "))
    }
}

impl CodegenBackend for SqlxBackend {
    fn name(&self) -> &str {
        "rust-sqlx"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "mysql", "mariadb", "sqlite", "redshift"]
    }

    fn file_header(&self) -> String {
        "#![allow(dead_code, unused_imports, clippy::needless_question_mark, clippy::redundant_closure)]".to_string()
    }

    fn apply_options(&mut self, options: &std::collections::HashMap<String, String>) -> Result<(), ScytheError> {
        reject_unknown_options(&["structs_only", "serde", "derive"], options)?;

        if let Some(value) = options.get("structs_only") {
            self.structs_only = parse_bool_option("structs_only", value)?;
        }
        if let Some(value) = options.get("serde") {
            self.serde = parse_bool_option("serde", value)?;
        }
        if let Some(value) = options.get("derive") {
            self.extra_derives = value.split(',').map(|s| s.trim().to_string()).collect();
        }
        Ok(())
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        _query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let mut out = String::new();

        let _ = writeln!(out, "{}", self.derive_line(&["Debug", "Clone", "sqlx::FromRow"]));
        let _ = writeln!(out, "pub struct {} {{", struct_name);

        for col in columns {
            let field_type = self.row_field_type(col);
            write_sqlx_rename_attr(&mut out, col);
            let _ = writeln!(out, "    pub {}: {},", col.field_name, field_type);
        }

        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        columns: &[ResolvedColumn],
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
        let sql =
            crate::sql_literal::escape_rust_string(&rewrite_sql_for_row_columns(&sql_raw, columns, &self.manifest));

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
                let _ = writeln!(out, "{}", self.derive_line(&["Debug", "Clone"]));
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
            QueryCommand::One => struct_name.to_string(),
            // `:opt` is "zero or one row", so it returns `Option<T>` and fetches
            // with `.fetch_optional`. Sharing `:one`'s arm produced generated code
            // that could not compile at all, not merely code that was too strict:
            // the return type said `{struct_name}` while `has_row_struct` below
            // excluded `Opt`, so the body emitted the anonymous-record
            // `sqlx::query!` instead of `sqlx::query_as!` and handed back a type
            // that was not `{struct_name}`. The declared type and the produced
            // type disagreed on every `:opt` query rust-sqlx has ever emitted.
            QueryCommand::Opt => format!("Option<{}>", struct_name),
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

        // `Opt` belongs here with `One`/`Many`: it returns rows, so it needs
        // `sqlx::query_as!` and the row struct. Omitting it is what made the
        // declared return type unreachable -- see the `Opt` arm above.
        let has_row_struct = matches!(
            analyzed.command,
            QueryCommand::One | QueryCommand::Many | QueryCommand::Opt
        );

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
            QueryCommand::One => ".fetch_one(pool)",
            QueryCommand::Opt => ".fetch_optional(pool)",
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

        let _ = writeln!(
            out,
            "{}",
            self.derive_line(&["Debug", "Clone", "PartialEq", "Eq", "sqlx::Type"])
        );
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

        let _ = writeln!(out, "{}", self.derive_line(&["Debug", "Clone", "sqlx::FromRow"]));
        let _ = writeln!(out, "pub struct {child_struct_name} {{");
        for col in child_columns {
            let field_type = self.row_field_type(col);
            write_sqlx_rename_attr(&mut out, col);
            let _ = writeln!(out, "    pub {}: {field_type},", col.field_name);
        }
        let _ = writeln!(out, "}}");

        let _ = writeln!(out);

        // ~keep The parent struct here is plain Debug/Clone, not sqlx::FromRow --
        // it is assembled field-by-field from `row.<field_name>` (see
        // generate_grouped_query_fn), never decoded by sqlx itself, so it has
        // no FromRow name-lookup to protect and gets no #[sqlx(rename)].
        let _ = writeln!(out, "{}", self.derive_line(&["Debug", "Clone"]));
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
        let sql = crate::sql_literal::escape_rust_string(&rewrite_sql_for_row_columns(
            &sql_raw,
            parent_columns.iter().chain(child_columns.iter()),
            &self.manifest,
        ));

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

        let struct_name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();

        let _ = writeln!(out, "{}", self.derive_line(&["Debug", "Clone", "sqlx::Type"]));
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
        // ~keep The plain form derives sqlx::Type only, which decodes an enum off
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

/// Emit a `#[sqlx(rename = "...")]` line ahead of a row-struct field whose
/// generated identifier differs from its SQL column name.
///
/// `sqlx::FromRow` (derived on every row struct this backend emits) looks a
/// column up *by the Rust field's own name* unless told otherwise.
/// `sanitize_field_names` (on in every rust-sqlx manifest) turns a column
/// like `my col` into the field `my_col` so the struct compiles, but without
/// this attribute that silently breaks the FromRow lookup: at runtime it
/// searches the row for a column literally named `my_col`, which does not
/// exist, producing a `ColumnNotFound` error (or, if a column happens to
/// share that sanitized name, a wrong value entirely) despite the generated
/// code compiling cleanly. The rename attribute keeps the FromRow lookup on
/// the original SQL name while the Rust-visible field stays sanitized.
fn write_sqlx_rename_attr(out: &mut String, col: &ResolvedColumn) {
    if col.field_name != col.name {
        let _ = writeln!(out, "    #[sqlx(rename = \"{}\")]", col.name);
    }
}

/// Alias every row column that needs one, folding an identifier rename and
/// an enum type override into a single `AS "..."` clause when a column
/// needs both.
///
/// Shared by `generate_query_fn` and `generate_grouped_query_fn`: both read
/// their row(s) through the untyped `sqlx::query!`/`query_as!` macros, and
/// `output::quote_query_as` (sqlx-macros-core 0.9.0, `query/output.rs:202`)
/// builds the record with `#out_ty { #ident: #var_name }`, where `#ident`
/// comes straight from `output::parse_ident` on the driver-reported column
/// name -- it never consults `#[sqlx(rename = "...")]` or `FromRow` at all,
/// so that attribute is inert on this path regardless of which of the two
/// functions is generating the query. Two independent things can make the
/// raw name unusable:
///
/// - **Identifier shape.** A column whose driver-reported name is not
///   itself a valid Rust identifier (`"my col"`) makes `parse_ident`
///   hard-error at compile time ("... is not a valid Rust identifier")
///   before anything else is checked. A column whose name *is* a valid
///   identifier but differs from this backend's own `field_name` (a case
///   difference, or `sanitize_field_names` reshaping `"widgetId"` into
///   `widget_id`) compiles, but against a struct-literal field that isn't
///   there -- `parse_ident` does not know about `field_name`'s convention,
///   it only validates shape.
/// - **Enum decoding.** sqlx cannot infer a custom enum's Rust type without
///   an explicit `: Type` override in the alias.
///
/// sqlx accepts only one alias clause per column, so a column needing both
/// gets `AS "field_name: Type"` rather than two rewrite passes fighting
/// over the same text.
fn rewrite_sql_for_row_columns<'a>(
    sql: &str,
    columns: impl IntoIterator<Item = &'a ResolvedColumn>,
    manifest: &BackendManifest,
) -> String {
    let mut result = sql.to_string();

    for col in columns {
        let type_override = col.neutral_type.strip_prefix("enum::").map(|enum_name| {
            let rust_type = enum_type_name(enum_name, &manifest.naming);
            if col.nullable {
                format!("Option<{}>", rust_type)
            } else {
                rust_type
            }
        });

        if col.field_name == col.name && type_override.is_none() {
            continue;
        }

        let Some(from_pos) = result.to_uppercase().find(" FROM ") else {
            continue;
        };
        let select_part = &result[..from_pos];
        let rest = &result[from_pos..];

        let alias_body = match &type_override {
            Some(ty) => format!("{}: {}", col.field_name, ty),
            None => col.field_name.clone(),
        };
        let alias = format!("\"{}\"", alias_body);

        let quoted_name = format!("\"{}\"", col.name);
        let new_select = if let Some(pos) = select_part.rfind(quoted_name.as_str()) {
            let mut selected = select_part.to_string();
            let replacement = format!("{} AS {}", quoted_name, alias);
            selected.replace_range(pos..pos + quoted_name.len(), &replacement);
            selected
        } else {
            let replacement = format!("{} AS {}", col.name, alias);
            replace_column_in_select(select_part, &col.name, &replacement)
        };
        result = format!("{}{}", new_select, rest);
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
    use scythe_core::analyzer::AnalyzedColumn;

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

    /// #103: before this, rust-sqlx inherited the `CodegenBackend` default
    /// `apply_options` (`Ok(())` for any map), so an unrecognized key was
    /// silently discarded here while the same typo was a hard error on
    /// every TypeScript backend.
    #[test]
    fn apply_options_rejects_unknown_key_with_invalid_config() {
        let mut backend = SqlxBackend::new("postgresql").unwrap();
        let err = backend
            .apply_options(&std::collections::HashMap::from([(
                "structs_onl".to_string(),
                "true".to_string(),
            )]))
            .expect_err("structs_onl is not a known rust-sqlx option");
        assert_eq!(err.code, ErrorCode::InvalidConfig);
        assert!(err.message.contains("structs_onl"), "{}", err.message);
        assert!(
            err.message.contains("structs_only"),
            "error should list the real option: {}",
            err.message
        );
    }

    /// Regression test for a release-blocking bug: `[sql.gen.rust] serde =
    /// true` / `derive = [...]` are documented as target-independent options
    /// under the legacy `[sql.gen.rust]` table (see
    /// `website/src/content/docs/guide/configuration.md`), and
    /// `resolve_gen_targets` in the CLI inserts them into the options map
    /// regardless of which `target` the user picked. rust-tokio-postgres
    /// handled both, but rust-sqlx only ever declared `structs_only`, so any
    /// `target = "sqlx"` config using `serde`/`derive` hard-errored with
    /// "unknown option 'serde'". All three options must coexist.
    #[test]
    fn serde_and_derive_options_apply_together_with_structs_only() {
        let mut backend = SqlxBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([
                ("serde".to_string(), "true".to_string()),
                ("derive".to_string(), "PartialEq, Eq".to_string()),
                ("structs_only".to_string(), "true".to_string()),
            ]))
            .unwrap();
        assert!(backend.serde);
        assert_eq!(backend.extra_derives, vec!["PartialEq".to_string(), "Eq".to_string()]);
        assert!(backend.structs_only);
    }

    /// Must fail before the fix: `derive_line` appended every `extra_derives` entry
    /// unconditionally, so a `derive` option repeating a name already in `base` (every row
    /// struct's base always includes "Debug" and "Clone") produced a literal duplicate
    /// derive token -- `#[derive(Debug, Clone, sqlx::FromRow, Debug)]` -- which is E0119
    /// ("conflicting implementations of trait `Debug`") in the generated file, not a
    /// harmless no-op.
    #[test]
    fn derive_line_dedupes_extra_derive_matching_a_base_derive() {
        let mut backend = SqlxBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "derive".to_string(),
                "Debug, PartialEq".to_string(),
            )]))
            .unwrap();
        let line = backend.derive_line(&["Debug", "Clone", "sqlx::FromRow"]);
        assert_eq!(line, "#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]");
    }

    /// The companion case: `derive` naming `serde::Serialize` while `serde = true` is also
    /// set must not duplicate the derive `serde` itself already inserted.
    #[test]
    fn derive_line_dedupes_extra_derive_matching_the_serde_derives() {
        let mut backend = SqlxBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([
                ("serde".to_string(), "true".to_string()),
                ("derive".to_string(), "serde::Serialize".to_string()),
            ]))
            .unwrap();
        let line = backend.derive_line(&["Debug", "Clone"]);
        assert_eq!(line, "#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]");
    }

    /// An unrecognized value must be reported, not silently treated as
    /// leaving `serde` disabled.
    #[test]
    fn serde_option_rejects_invalid_value() {
        let mut backend = SqlxBackend::new("postgresql").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "serde".to_string(),
            "maybe".to_string(),
        )]));
        assert!(result.is_err(), "expected 'maybe' to be rejected");
    }

    /// The regression itself: applying `serde`/`derive` must not just avoid
    /// erroring, the emitted row struct must actually carry the serde and
    /// extra derives alongside sqlx's own `sqlx::FromRow`.
    #[test]
    fn serde_and_derive_options_generate_row_struct_with_serde_derive() {
        let mut backend = SqlxBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([
                ("serde".to_string(), "true".to_string()),
                ("derive".to_string(), "PartialEq".to_string()),
            ]))
            .unwrap();

        let query = AnalyzedQuery::build(|aq| {
            aq.name = "GetUser".to_string();
            aq.command = QueryCommand::Opt;
            aq.sql = "SELECT id, name FROM users".to_string();
            aq.columns = vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    ..Default::default()
                },
            ];
        });

        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct
                .contains("#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize, PartialEq)]"),
            "row struct must carry the base derives plus serde and the extra derive; got:\n{row_struct}"
        );
    }
}
