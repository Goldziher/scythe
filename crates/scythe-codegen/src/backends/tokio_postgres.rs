use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{composite_type_name, enum_type_name, enum_variant_name, fn_name, to_snake_case};
use scythe_backend::types::resolve_type;
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::GeneratedCode;
use crate::backend_options::reject_unknown_options;
use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};
use crate::backends::typescript_common::parse_bool_option;

/// Default embedded manifest TOML for rust-tokio-postgres, used as fallback.
const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/rust-tokio-postgres.toml");
const DEFAULT_MANIFEST_REDSHIFT: &str = include_str!("../../manifests/rust-tokio-postgres.redshift.toml");

/// Hand-rolled `FromSql`/`ToSql` for the `range = "PgRange<{T}>"` container mapping.
///
/// `postgres-types` 0.2.14 ships no `Range<T>` with a `FromSql`/`ToSql` impl (unlike
/// `sqlx::postgres::types::PgRange<T>`, which this mirrors in shape), so the backend defines its
/// own -- on top of `postgres_protocol::types::{range_from_sql, range_to_sql}`, the binary
/// wire-format primitives `postgres-types` itself uses internally for arrays (read from the
/// vendored crate source). Emitted once per output file, only when a generated fragment actually
/// names `PgRange<`, via `file_header_for_results` below.
///
/// `Empty` is kept distinct from `Range { start: Bound::Unbounded, end: Bound::Unbounded }`: an
/// empty range and a fully-unbounded one are different values on the wire, and collapsing them
/// (as some other clients do) would make an empty-range column decode as if it contained
/// everything.
const PG_RANGE_SUPPORT: &str = r#"#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PgRange<T> {
    Empty,
    Range {
        start: std::ops::Bound<T>,
        end: std::ops::Bound<T>,
    },
}

impl<'a, T> tokio_postgres::types::FromSql<'a> for PgRange<T>
where
    T: tokio_postgres::types::FromSql<'a>,
{
    fn from_sql(
        ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let element_type = match ty.kind() {
            tokio_postgres::types::Kind::Range(element) => element,
            _ => return Err("PgRange::from_sql called on a non-range type".into()),
        };
        match postgres_protocol::types::range_from_sql(raw)? {
            postgres_protocol::types::Range::Empty => Ok(PgRange::Empty),
            postgres_protocol::types::Range::Nonempty(lower, upper) => Ok(PgRange::Range {
                start: pg_range_bound_from_sql(element_type, lower)?,
                end: pg_range_bound_from_sql(element_type, upper)?,
            }),
        }
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        matches!(ty.kind(), tokio_postgres::types::Kind::Range(element) if T::accepts(element))
    }
}

fn pg_range_bound_from_sql<'a, T>(
    element_type: &tokio_postgres::types::Type,
    bound: postgres_protocol::types::RangeBound<Option<&'a [u8]>>,
) -> Result<std::ops::Bound<T>, Box<dyn std::error::Error + Sync + Send>>
where
    T: tokio_postgres::types::FromSql<'a>,
{
    match bound {
        postgres_protocol::types::RangeBound::Inclusive(raw) => {
            Ok(std::ops::Bound::Included(T::from_sql_nullable(element_type, raw)?))
        }
        postgres_protocol::types::RangeBound::Exclusive(raw) => {
            Ok(std::ops::Bound::Excluded(T::from_sql_nullable(element_type, raw)?))
        }
        postgres_protocol::types::RangeBound::Unbounded => Ok(std::ops::Bound::Unbounded),
    }
}

impl<T> tokio_postgres::types::ToSql for PgRange<T>
where
    T: tokio_postgres::types::ToSql,
{
    fn to_sql(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        let element_type = match ty.kind() {
            tokio_postgres::types::Kind::Range(element) => element,
            _ => return Err("PgRange::to_sql called on a non-range type".into()),
        };
        match self {
            PgRange::Empty => postgres_protocol::types::empty_range_to_sql(out),
            PgRange::Range { start, end } => {
                postgres_protocol::types::range_to_sql(
                    |buf| pg_range_bound_to_sql(start, element_type, buf),
                    |buf| pg_range_bound_to_sql(end, element_type, buf),
                    out,
                )?;
            }
        }
        Ok(tokio_postgres::types::IsNull::No)
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        matches!(ty.kind(), tokio_postgres::types::Kind::Range(element) if T::accepts(element))
    }

    tokio_postgres::types::to_sql_checked!();
}

fn pg_range_bound_to_sql<T>(
    bound: &std::ops::Bound<T>,
    element_type: &tokio_postgres::types::Type,
    buf: &mut tokio_postgres::types::private::BytesMut,
) -> Result<postgres_protocol::types::RangeBound<postgres_protocol::IsNull>, Box<dyn std::error::Error + Sync + Send>>
where
    T: tokio_postgres::types::ToSql,
{
    match bound {
        std::ops::Bound::Included(value) => Ok(postgres_protocol::types::RangeBound::Inclusive(
            pg_range_element_is_null(value, element_type, buf)?,
        )),
        std::ops::Bound::Excluded(value) => Ok(postgres_protocol::types::RangeBound::Exclusive(
            pg_range_element_is_null(value, element_type, buf)?,
        )),
        std::ops::Bound::Unbounded => Ok(postgres_protocol::types::RangeBound::Unbounded),
    }
}

fn pg_range_element_is_null<T: tokio_postgres::types::ToSql>(
    value: &T,
    element_type: &tokio_postgres::types::Type,
    buf: &mut tokio_postgres::types::private::BytesMut,
) -> Result<postgres_protocol::IsNull, Box<dyn std::error::Error + Sync + Send>> {
    Ok(match value.to_sql(element_type, buf)? {
        tokio_postgres::types::IsNull::No => postgres_protocol::IsNull::No,
        tokio_postgres::types::IsNull::Yes => postgres_protocol::IsNull::Yes,
    })
}"#;

/// Whether any generated fragment in `generated` contains `needle` -- used to gate emitting
/// [`PG_RANGE_SUPPORT`] on a file actually naming `PgRange<`, so a file with no range column
/// does not carry an unused type (`dead_code` is already allowed file-wide, but there is no
/// reason to ship the definition where nothing reaches it).
fn generated_code_uses(generated: &[GeneratedCode], needle: &str) -> bool {
    generated.iter().any(|code| {
        [
            code.enum_def.as_deref(),
            code.model_struct.as_deref(),
            code.row_struct.as_deref(),
            code.query_fn.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|fragment| fragment.contains(needle))
            || code.nested_struct_defs.iter().any(|def| def.code.contains(needle))
    })
}

/// TokioPostgresBackend generates Rust code targeting the tokio-postgres crate.
pub struct TokioPostgresBackend {
    manifest: BackendManifest,
    serde: bool,
    extra_derives: Vec<String>,
    /// Whether this engine's manifest declares the `json_nested` container
    /// and its server actually has `json_agg`. See
    /// [`crate::backends::engine_supports_nested_aggregates`].
    nested_aggregates: bool,
}

impl TokioPostgresBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_TOML,
            "redshift" => DEFAULT_MANIFEST_REDSHIFT,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!(
                        "rust-tokio-postgres only supports PostgreSQL/Redshift, got engine '{}'",
                        engine
                    ),
                ));
            }
        };
        let manifest = super::parse_manifest(default_toml)?;
        Ok(Self {
            manifest,
            serde: false,
            extra_derives: Vec::new(),
            nested_aggregates: super::engine_supports_nested_aggregates(engine),
        })
    }

    /// Build the derive line for structs, incorporating serde and extra derives.
    fn struct_derives(&self) -> String {
        let mut derives = vec!["Debug", "Clone"];
        if self.serde {
            derives.push("serde::Serialize");
            derives.push("serde::Deserialize");
        }
        for d in &self.extra_derives {
            derives.push(d);
        }
        format!("#[derive({})]", derives.join(", "))
    }

    /// Build the derive line for enums (includes PartialEq, Eq).
    fn enum_derives(&self) -> String {
        let mut derives = vec!["Debug", "Clone", "PartialEq", "Eq"];
        if self.serde {
            derives.push("serde::Serialize");
            derives.push("serde::Deserialize");
        }
        for d in &self.extra_derives {
            derives.push(d);
        }
        format!("#[derive({})]", derives.join(", "))
    }

    /// Build the derive line for composite structs.
    ///
    /// ~keep Unlike a row struct (read off `tokio_postgres::Row` field-by-field via
    /// `row.get`), a composite value crosses the wire as a single column and needs its
    /// own `ToSql`/`FromSql` impl. `postgres_types::{ToSql, FromSql}` (re-exported from
    /// `postgres-derive` behind postgres-types' `derive` feature) supply exactly that;
    /// see the crate's composite/naming doc examples for the derive plus
    /// `#[postgres(name = "...")]` shape this mirrors.
    fn composite_derives(&self) -> String {
        let mut derives = vec!["Debug", "Clone", "postgres_types::ToSql", "postgres_types::FromSql"];
        if self.serde {
            derives.push("serde::Serialize");
            derives.push("serde::Deserialize");
        }
        for d in &self.extra_derives {
            derives.push(d);
        }
        format!("#[derive({})]", derives.join(", "))
    }
}

const CLIENT_PARAM: &str = "client: &(impl tokio_postgres::GenericClient + Sync)";
const ERROR_TYPE: &str = "tokio_postgres::Error";

impl CodegenBackend for TokioPostgresBackend {
    fn name(&self) -> &str {
        "rust-tokio-postgres"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "redshift"]
    }

    fn file_header(&self) -> String {
        "#![allow(dead_code, unused_imports, clippy::needless_question_mark, clippy::redundant_closure)]".to_string()
    }

    fn file_header_for_results(&self, generated: &[GeneratedCode]) -> String {
        if generated_code_uses(generated, "PgRange<") {
            format!("{}\n\n{}", self.file_header(), PG_RANGE_SUPPORT)
        } else {
            self.file_header()
        }
    }

    fn apply_options(&mut self, options: &std::collections::HashMap<String, String>) -> Result<(), ScytheError> {
        reject_unknown_options(&["serde", "derive"], options)?;

        if let Some(val) = options.get("serde") {
            self.serde = parse_bool_option("serde", val)?;
        }
        if let Some(val) = options.get("derive") {
            self.extra_derives = val.split(',').map(|s| s.trim().to_string()).collect();
        }
        Ok(())
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        _query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        generate_struct_with_from_row(struct_name, columns, &self.struct_derives())
    }

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        _columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        if let Some(ref msg) = analyzed.deprecated {
            let _ = writeln!(out, "#[deprecated(note = \"{}\")]", msg);
        }

        let mut param_parts: Vec<String> = vec![CLIENT_PARAM.to_string()];
        for param in params {
            param_parts.push(format!("{}: {}", param.field_name, param.borrowed_type));
        }

        let sql = crate::sql_literal::rust_raw_string_literal(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        if matches!(analyzed.command, QueryCommand::Batch) {
            let batch_fn_name = format!("{}_batch", func_name);

            if params.len() > 1 {
                let params_struct_name = format!("{}BatchParams", struct_name);
                let _ = writeln!(out, "{}", self.struct_derives());
                let _ = writeln!(out, "pub struct {} {{", params_struct_name);
                for param in params {
                    let _ = writeln!(out, "    pub {}: {},", param.field_name, param.full_type);
                }
                let _ = writeln!(out, "}}");
                let _ = writeln!(out);
                let _ = writeln!(
                    out,
                    "pub async fn {}({}, items: &[{}]) -> Result<(), {}> {{",
                    batch_fn_name, CLIENT_PARAM, params_struct_name, ERROR_TYPE
                );
                let _ = writeln!(out, "    let stmt = client.prepare({}).await?;", sql);
                let _ = writeln!(out, "    for item in items {{");
                let refs: Vec<String> = params
                    .iter()
                    .map(|p| {
                        if p.neutral_type.starts_with("enum::") {
                            format!("&item.{}.to_string()", p.field_name)
                        } else {
                            format!("&item.{}", p.field_name)
                        }
                    })
                    .collect();
                let _ = writeln!(out, "        client.execute(&stmt, &[{}]).await?;", refs.join(", "));
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "    Ok(())");
            } else if params.len() == 1 {
                let param = &params[0];
                let _ = writeln!(
                    out,
                    "pub async fn {}({}, items: &[{}]) -> Result<(), {}> {{",
                    batch_fn_name, CLIENT_PARAM, param.full_type, ERROR_TYPE
                );
                let _ = writeln!(out, "    let stmt = client.prepare({}).await?;", sql);
                let _ = writeln!(out, "    for item in items {{");
                let _ = writeln!(out, "        client.execute(&stmt, &[item]).await?;");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "    Ok(())");
            } else {
                let _ = writeln!(
                    out,
                    "pub async fn {}({}, count: usize) -> Result<(), {}> {{",
                    batch_fn_name, CLIENT_PARAM, ERROR_TYPE
                );
                let _ = writeln!(out, "    let stmt = client.prepare({}).await?;", sql);
                let _ = writeln!(out, "    for _ in 0..count {{");
                let _ = writeln!(out, "        client.execute(&stmt, &[]).await?;");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "    Ok(())");
            }

            let _ = write!(out, "}}");
            return Ok(out);
        }

        let return_type = match &analyzed.command {
            QueryCommand::One => struct_name.to_string(),
            QueryCommand::Opt => format!("Option<{}>", struct_name),
            QueryCommand::Many => format!("Vec<{}>", struct_name),
            QueryCommand::Exec => "()".to_string(),
            QueryCommand::ExecResult => "u64".to_string(),
            QueryCommand::ExecRows => "u64".to_string(),
            QueryCommand::Batch => unreachable!(),
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
        };

        let _ = writeln!(
            out,
            "pub async fn {}({}) -> Result<{}, {}> {{",
            func_name,
            param_parts.join(", "),
            return_type,
            ERROR_TYPE
        );

        let param_refs: String = if params.is_empty() {
            "&[]".to_string()
        } else {
            let refs: Vec<String> = params.iter().map(|p| format!("&{}", p.field_name)).collect();
            format!("&[{}]", refs.join(", "))
        };

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(out, "    let row = client.query_one({}, {}).await?;", sql, param_refs);
                let _ = writeln!(out, "    Ok({}::from_row(&row))", struct_name);
            }
            QueryCommand::Opt => {
                let _ = writeln!(out, "    let row = client.query_opt({}, {}).await?;", sql, param_refs);
                let _ = writeln!(out, "    Ok(row.as_ref().map({}::from_row))", struct_name);
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "    let rows = client.query({}, {}).await?;", sql, param_refs);
                let _ = writeln!(out, "    Ok(rows.iter().map({}::from_row).collect())", struct_name);
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "    client.execute({}, {}).await?;", sql, param_refs);
                let _ = writeln!(out, "    Ok(())");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "    let rows_affected = client.execute({}, {}).await?;",
                    sql, param_refs
                );
                let _ = writeln!(out, "    Ok(rows_affected)");
            }
            QueryCommand::Batch => unreachable!(),
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
        }

        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::with_capacity(512);

        let _ = writeln!(out, "{}", self.enum_derives());
        let _ = writeln!(out, "pub enum {} {{", type_name);
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "    {},", variant);
        }
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);

        let _ = writeln!(out, "impl std::fmt::Display for {} {{", type_name);
        let _ = writeln!(
            out,
            "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
        );
        let _ = writeln!(out, "        match self {{");
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(
                out,
                "            {}::{} => write!(f, \"{}\"),",
                type_name, variant, value
            );
        }
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);

        let _ = writeln!(out, "impl std::str::FromStr for {} {{", type_name);
        let _ = writeln!(out, "    type Err = String;");
        let _ = writeln!(out, "    fn from_str(s: &str) -> Result<Self, Self::Err> {{");
        let _ = writeln!(out, "        match s {{");
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "            \"{}\" => Ok({}::{}),", value, type_name, variant);
        }
        let _ = writeln!(out, "            _ => Err(format!(\"unknown variant: {{}}\", s)),");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);

        let _ = writeln!(out, "impl<'a> tokio_postgres::types::FromSql<'a> for {} {{", type_name);
        let _ = writeln!(
            out,
            "    fn from_sql(ty: &tokio_postgres::types::Type, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {{"
        );
        let _ = writeln!(
            out,
            "        let s = <&str as tokio_postgres::types::FromSql>::from_sql(ty, raw)?;"
        );
        let _ = writeln!(out, "        s.parse::<{}>().map_err(|e| e.into())", type_name);
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out);
        let _ = writeln!(out, "    fn accepts(ty: &tokio_postgres::types::Type) -> bool {{");
        let _ = writeln!(
            out,
            "        ty.name() == \"{}\" || <&str as tokio_postgres::types::FromSql>::accepts(ty)",
            enum_info.sql_name
        );
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);

        let _ = writeln!(out, "impl tokio_postgres::types::ToSql for {} {{", type_name);
        let _ = writeln!(
            out,
            "    fn to_sql(&self, ty: &tokio_postgres::types::Type, out: &mut tokio_postgres::types::private::BytesMut) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {{"
        );
        let _ = writeln!(out, "        self.to_string().to_sql(ty, out)");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out);
        let _ = writeln!(out, "    fn accepts(ty: &tokio_postgres::types::Type) -> bool {{");
        let _ = writeln!(
            out,
            "        ty.name() == \"{}\" || <String as tokio_postgres::types::ToSql>::accepts(ty)",
            enum_info.sql_name
        );
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out);
        let _ = writeln!(out, "    tokio_postgres::types::to_sql_checked!();");
        let _ = writeln!(out, "}}");

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

        out.push_str(&generate_struct_with_from_row(
            child_struct_name,
            child_columns,
            &self.struct_derives(),
        )?);
        let _ = writeln!(out);
        let _ = writeln!(out);

        let _ = writeln!(out, "{}", self.struct_derives());
        let _ = writeln!(out, "pub struct {} {{", parent_struct_name);
        for col in parent_columns {
            let _ = writeln!(out, "    pub {}: {},", col.field_name, col.full_type);
        }
        let _ = writeln!(out, "    pub children: Vec<{}>,", child_struct_name);
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
        let params = request.params;
        let key_column = request.key_column;

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let key_field = to_snake_case(key_column);
        let mut out = String::new();

        if let Some(ref msg) = analyzed.deprecated {
            let _ = writeln!(out, "#[deprecated(note = \"{msg}\")]");
        }

        let mut param_parts: Vec<String> = vec![CLIENT_PARAM.to_string()];
        for param in params {
            param_parts.push(format!("{}: {}", param.field_name, param.borrowed_type));
        }

        let sql = crate::sql_literal::rust_raw_string_literal(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        let param_refs: String = if params.is_empty() {
            "&[]".to_string()
        } else {
            let refs: Vec<String> = params.iter().map(|p| format!("&{}", p.field_name)).collect();
            format!("&[{}]", refs.join(", "))
        };

        let key_col_type = parent_columns
            .iter()
            .find(|c| c.name == key_column || c.field_name == key_field)
            .map(|c| c.full_type.as_str())
            .unwrap_or("i64");

        let _ = writeln!(
            out,
            "pub async fn {}({}) -> Result<Vec<{}>, {}> {{",
            func_name,
            param_parts.join(", "),
            parent_struct_name,
            ERROR_TYPE
        );

        let _ = writeln!(out, "    let rows = client.query({}, {}).await?;", sql, param_refs);
        let _ = writeln!(out, "    let mut result: Vec<{}> = Vec::new();", parent_struct_name);
        let _ = writeln!(out, "    for row in &rows {{");

        for col in parent_columns {
            let _ = writeln!(
                out,
                "        let {}: {} = row.get(\"{}\");",
                col.field_name, col.full_type, col.name
            );
        }

        let _ = writeln!(out, "        let key: {key_col_type} = {key_field}.clone();");

        let _ = writeln!(out, "        let child = {}::from_row(row);", child_struct_name);

        let _ = writeln!(
            out,
            "        if let Some(parent) = result.iter_mut().rev().find(|p| p.{key_field} == key) {{"
        );
        let _ = writeln!(out, "            parent.children.push(child);");
        let _ = writeln!(out, "        }} else {{");
        let _ = writeln!(out, "            result.push({} {{", parent_struct_name);
        for col in parent_columns {
            let _ = writeln!(out, "                {},", col.field_name);
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
        let struct_name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();

        let _ = writeln!(out, "{}", self.composite_derives());
        let _ = writeln!(out, "#[postgres(name = \"{}\")]", composite.sql_name);
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

        // ~keep Deliberately not self.struct_derives(): the `serde` option decides
        // whether the *row* struct (built by from_row/row.get, never
        // JSON-decoded) opts into serde, which is a separate question from
        // this struct's own unconditional need for both serde traits --
        // `json_nested<T>` resolves to `postgres_types::Json<T>`, whose
        // `FromSql` is bounded on `T: Deserialize`. See
        // `generate_nested_rust_struct` for the rest.
        Ok(Some(super::sqlx::generate_nested_rust_struct(nested, &self.manifest)?))
    }

    fn generate_enum_def_for_nested(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        // ~keep enum_derives() adds serde only when the `serde` option is on, but
        // a nested struct needs its field types `Deserialize` regardless:
        // that struct is decoded from JSON, not off the wire.
        let base = self.generate_enum_def(enum_info)?;
        Ok(super::sqlx::add_serde_to_enum(&base, enum_info, &self.manifest))
    }

    fn generate_composite_def_for_nested(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let base = self.generate_composite_def(composite)?;
        Ok(super::sqlx::add_serde_to_first_derive(&base))
    }
}

/// Generate a struct with a `from_row` method for tokio-postgres.
///
/// When no enum columns exist, `from_row` is infallible (panics on type mismatch
/// just like `row.get()` does). When enum columns are present, `from_row` returns
/// `Self` but panics on invalid enum values — matching tokio-postgres conventions.
fn generate_struct_with_from_row(
    struct_name: &str,
    columns: &[ResolvedColumn],
    derive_line: &str,
) -> Result<String, ScytheError> {
    let mut out = String::new();

    let _ = writeln!(out, "{}", derive_line);
    let _ = writeln!(out, "pub struct {} {{", struct_name);
    for col in columns {
        let _ = writeln!(out, "    pub {}: {},", col.field_name, col.full_type);
    }
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    let _ = writeln!(out, "impl {} {{", struct_name);
    let _ = writeln!(out, "    pub fn from_row(row: &tokio_postgres::Row) -> Self {{");
    let _ = writeln!(out, "        Self {{");
    for col in columns {
        let _ = writeln!(out, "            {}: row.get(\"{}\"),", col.field_name, col.name);
    }
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");
    let _ = write!(out, "}}");

    Ok(out)
}

#[cfg(test)]
mod tests {
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, GroupByConfig};
    use scythe_core::parser::QueryCommand;

    use super::TokioPostgresBackend;
    use crate::generate_with_backend;

    fn make_grouped_query() -> AnalyzedQuery {
        let parent_cols = vec![
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
            AnalyzedColumn {
                name: "email".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                ..Default::default()
            },
        ];
        let child_cols = vec![
            AnalyzedColumn {
                name: "order_id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "total".to_string(),
                neutral_type: "decimal".to_string(),
                nullable: true,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "order_date".to_string(),
                neutral_type: "datetime".to_string(),
                nullable: false,
                ..Default::default()
            },
        ];
        let all_cols = [parent_cols.clone(), child_cols.clone()].concat();
        AnalyzedQuery::build(|aq| {
            aq.name = "GetUsersWithOrders".to_string();
            aq.command = QueryCommand::Grouped;
            aq.sql = "-- @name GetUsersWithOrders\n-- @returns :grouped\n-- @group_by users.id\n\
                  SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\n\
                  FROM users u\n\
                  JOIN orders o ON o.user_id = u.id"
                .to_string();
            aq.columns = all_cols;
            aq.params = vec![];
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = vec![];
            aq.enums = vec![];
            aq.optional_params = vec![];
            aq.group_by = Some(GroupByConfig {
                table: "users".to_string(),
                key_column: "id".to_string(),
                parent_columns: parent_cols,
                child_columns: child_cols,
            });
            aq.custom = vec![];
        })
    }

    #[test]
    fn test_grouped_tokio_postgres_structs() {
        let backend = TokioPostgresBackend::new("postgresql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &backend).unwrap();

        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct.contains("pub struct GetUsersWithOrdersChildRow"),
            "missing child struct; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub order_id: i32"),
            "child struct missing order_id; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub struct GetUsersWithOrdersRow"),
            "missing parent struct; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub id: i32"),
            "parent struct missing id; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub name: String"),
            "parent struct missing name; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub children: Vec<GetUsersWithOrdersChildRow>"),
            "parent struct missing children field; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("pub struct GetUsersWithOrdersRow").unwrap();
        assert!(child_pos < parent_pos, "child struct must appear before parent struct");

        assert!(
            row_struct.contains("tokio_postgres::Row"),
            "child struct should include from_row"
        );
        assert!(result.model_struct.is_none(), "grouped must not produce a model_struct");
    }

    #[test]
    fn test_grouped_tokio_postgres_query_fn() {
        let backend = TokioPostgresBackend::new("postgresql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("pub async fn get_users_with_orders("),
            "missing fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("-> Result<Vec<GetUsersWithOrdersRow>, tokio_postgres::Error>"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("client.query("),
            "fn must use client.query; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersChildRow::from_row(row)"),
            "fn must call child from_row; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("children: vec![child]"),
            "fn must initialize children vec; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("parent.children.push(child)"),
            "fn must fold child into existing parent; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Ok(result)"),
            "fn must return result; got:\n{query_fn}"
        );
    }

    #[test]
    fn serde_option_applies_when_true() {
        use crate::backend_trait::CodegenBackend;

        let mut backend = TokioPostgresBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "serde".to_string(),
                "true".to_string(),
            )]))
            .unwrap();
        assert!(backend.serde);
    }

    /// An unrecognized value must be reported, not silently treated as
    /// leaving `serde` disabled.
    #[test]
    fn serde_option_rejects_invalid_value() {
        use crate::backend_trait::CodegenBackend;

        let mut backend = TokioPostgresBackend::new("postgresql").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "serde".to_string(),
            "maybe".to_string(),
        )]));
        assert!(result.is_err(), "expected 'maybe' to be rejected");
    }
}
