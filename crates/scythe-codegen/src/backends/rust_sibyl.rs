use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case, to_snake_case,
};
use scythe_backend::types::resolve_type;
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};
use crate::singularize;

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/rust-sibyl.toml");

pub struct RustSibylBackend {
    manifest: BackendManifest,
}

impl RustSibylBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "oracle" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("rust-sibyl only supports Oracle, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::parse_manifest(DEFAULT_MANIFEST_TOML)?;
        Ok(Self { manifest })
    }

    /// Build the sibyl 0.7 `execute`/`query` args expression for IN params.
    /// Single param: `(":NAME", val)` — no outer tuple needed (it IS a (&str, T)).
    /// Multiple params: `((":A", a), (":B", b))`.
    /// Zero params: `()`.
    fn build_in_args(params: &[ResolvedParam]) -> String {
        match params.len() {
            0 => "()".to_string(),
            1 => format!(
                "(\"{}\", {})",
                Self::named_placeholder(&params[0].field_name),
                params[0].field_name
            ),
            _ => {
                let pairs: Vec<String> = params
                    .iter()
                    .map(|p| format!("(\"{}\", {})", Self::named_placeholder(&p.field_name), p.field_name))
                    .collect();
                format!("({})", pairs.join(", "))
            }
        }
    }

    /// `:PARAM_NAME` — named placeholder used in SQL for IN params.
    fn named_placeholder(field_name: &str) -> String {
        format!(":{}", field_name.to_uppercase())
    }

    /// `:OUT_COL_NAME` — named placeholder used in SQL for RETURNING INTO OUT params.
    fn out_named_placeholder(field_name: &str) -> String {
        format!(":OUT_{}", field_name.to_uppercase())
    }

    /// Rewrite positional `:N` placeholders in SQL to named `:PARAM_NAME` form.
    fn sql_with_named_params(sql: &str, params: &[ResolvedParam]) -> String {
        let mut result = sql.to_string();
        for (i, p) in params.iter().enumerate().rev() {
            let positional = format!(":{}", i + 1);
            let named = format!(":{}", p.field_name.to_uppercase());
            result = result.replace(&positional, &named);
        }
        result
    }

    /// Build the RETURNING INTO clause and the full SQL for a RETURNING DML.
    /// Returns the full SQL including RETURNING ... INTO :OUT_COL_NAME ...
    fn sql_with_returning(base_sql: &str, columns: &[ResolvedColumn]) -> String {
        let out_names: Vec<String> = columns
            .iter()
            .map(|c| format!(":OUT_{}", c.field_name.to_uppercase()))
            .collect();
        let upper = base_sql.to_uppercase();
        if let Some(ret_pos) = upper.find("RETURNING") {
            let prefix = &base_sql[..ret_pos];
            let rest = &base_sql[ret_pos + "RETURNING".len()..];
            let col_list = if let Some(into_pos) = rest.to_uppercase().find(" INTO ") {
                rest[..into_pos].trim()
            } else {
                rest.trim()
            };
            format!("{}RETURNING {} INTO {}", prefix, col_list, out_names.join(", "))
        } else {
            base_sql.to_string()
        }
    }

    /// Emit the variable declaration for a RETURNING INTO out variable.
    /// sibyl 0.7 accepts `&mut` primitive types (i64, f64, String) and sibyl types (Date, Number).
    /// For dates: declare a `Date::new(session)` placeholder.
    /// For int64: use i64 directly (Oracle NUMBER → SQLT_INT).
    /// For float/decimal: use f64 (Oracle NUMBER → SQLT_BDOUBLE).
    fn emit_out_var_decl_typed(col: &ResolvedColumn) -> String {
        match col.neutral_type.as_str() {
            "int16" | "int32" | "int64" => {
                format!("    let mut out_{}: i64 = 0;", col.field_name)
            }
            "float32" | "float64" | "decimal" => {
                format!("    let mut out_{}: f64 = 0.0;", col.field_name)
            }
            "date" | "datetime" | "datetime_tz" => {
                format!("    let mut out_{} = Date::new(session);", col.field_name)
            }
            _ => {
                // sibyl sizes the OCI out-buffer from the String's capacity; a zero-capacity
                // String silently truncates the returned value. 4000 bytes covers Oracle's
                // standard VARCHAR2 maximum.
                format!("    let mut out_{} = String::with_capacity(4000);", col.field_name)
            }
        }
    }

    /// For RETURNING args, get the `&mut var` expression for the out column.
    fn out_var_ref_for_returning(col: &ResolvedColumn) -> String {
        format!("&mut out_{}", col.field_name)
    }

    /// Emit the post-execute conversion from the `out_*` var to the struct field type.
    fn emit_out_var_conversion(col: &ResolvedColumn) -> String {
        match col.neutral_type.as_str() {
            "int16" => format!("    let {} = out_{} as i16;", col.field_name, col.field_name),
            "int32" => format!("    let {} = out_{} as i32;", col.field_name, col.field_name),
            "int64" => format!("    let {} = out_{};", col.field_name, col.field_name),
            "float32" => format!("    let {} = out_{} as f32;", col.field_name, col.field_name),
            "float64" | "decimal" => {
                format!("    let {} = out_{};", col.field_name, col.field_name)
            }
            "date" | "datetime" | "datetime_tz" => {
                format!(
                    "    let {name} = {{ let (y, mo, d, h, mi, s) = out_{name}.date_and_time(); chrono::NaiveDate::from_ymd_opt(y as i32, mo as u32, d as u32).and_then(|dt| dt.and_hms_opt(h as u32, mi as u32, s as u32)).expect(\"invalid date from Oracle\") }};",
                    name = col.field_name
                )
            }
            _ => {
                if col.nullable {
                    format!(
                        "    let {name} = if stmt.is_null(\"{placeholder}\")? {{ None }} else {{ Some(out_{name}) }};",
                        name = col.field_name,
                        placeholder = Self::out_named_placeholder(&col.field_name)
                    )
                } else {
                    format!("    let {} = out_{};", col.field_name, col.field_name)
                }
            }
        }
    }

    /// Emit the row.get() call for a SELECT column.
    /// sibyl 0.7's FromSql supports: String, &str, integers (via Integer trait), f32, f64,
    /// Date<'_>, Timestamp<'_>, etc. — but NOT chrono::NaiveDateTime.
    /// For date/datetime: get as Date<'_> then convert via date_and_time().
    /// decimal maps to f64 in the manifest (OCI NUMBER → SQLT_BDOUBLE), so it's handled as float.
    ///
    /// Oracle LOB columns need special handling: sibyl's `FromSql<String>` only matches
    /// `ColumnBuffer::Text` (VARCHAR2/CHAR) — a CLOB/NCLOB/BLOB/BFILE column's buffer is a
    /// LOB locator (`ColumnBuffer::CLOB`/`ColumnBuffer::BLOB`/...) and reading it via
    /// `row.get::<String, _>(i)` fails at runtime with `Interface("cannot return as a
    /// String")`. Every one of these four types must be fetched as a LOB locator and its
    /// content read explicitly. `neutral_type` alone can't distinguish this (CLOB and
    /// VARCHAR2 both resolve to `"string"`; BLOB and BFILE both resolve to `"bytes"`), so
    /// this matches on `sql_type`, the raw source SQL type.
    fn emit_row_get(col: &ResolvedColumn, index: usize, indent: &str) -> String {
        match col.sql_type.as_str() {
            "clob" | "nclob" => return Self::emit_row_get_lob(col, index, indent, LobKind::Text),
            "blob" => return Self::emit_row_get_lob(col, index, indent, LobKind::Blob),
            "bfile" => return Self::emit_row_get_lob(col, index, indent, LobKind::BFile),
            _ => {}
        }
        match col.neutral_type.as_str() {
            "date" | "datetime" | "datetime_tz" => {
                if col.nullable {
                    format!(
                        "{indent}let {name}: {ty} = row.get::<Option<Date<'_>>, _>({i})?.map(|d| {{ let (y, mo, d2, h, mi, s) = d.date_and_time(); chrono::NaiveDate::from_ymd_opt(y as i32, mo as u32, d2 as u32).and_then(|dt| dt.and_hms_opt(h as u32, mi as u32, s as u32)).expect(\"invalid date from Oracle\") }});",
                        indent = indent,
                        name = col.field_name,
                        i = index,
                        ty = col.full_type
                    )
                } else {
                    format!(
                        "{indent}let {name}_date: Date<'_> = row.get({i})?;\n\
                         {indent}let {name}: {ty} = {{ let (y, mo, d, h, mi, s) = {name}_date.date_and_time(); chrono::NaiveDate::from_ymd_opt(y as i32, mo as u32, d as u32).and_then(|dt| dt.and_hms_opt(h as u32, mi as u32, s as u32)).expect(\"invalid date from Oracle\") }};",
                        indent = indent,
                        name = col.field_name,
                        i = index,
                        ty = col.full_type
                    )
                }
            }
            _ => {
                format!(
                    "{indent}let {name}: {ty} = row.get({i})?;",
                    indent = indent,
                    name = col.field_name,
                    i = index,
                    ty = col.full_type
                )
            }
        }
    }

    /// Emit the row.get() call for an Oracle LOB column (CLOB, NCLOB, BLOB, or BFILE).
    /// Fetches the LOB locator, then reads its full content via a loop that keeps
    /// calling `LOB::read` until the accumulated read count reaches the LOB's reported
    /// length.
    ///
    /// sibyl's `LOB::read(offset, len, buf)` issues a single `OCI_ONE_PIECE` OCI call and
    /// returns the number of characters (CLOB/NCLOB) or bytes (BLOB/BFILE) actually read —
    /// sibyl's own doc example demonstrates this can be *less* than `len` (it returns
    /// whatever is available up to the LOB's current end, e.g. when the LOB shrinks
    /// between the `len()` call and the `read()` call, or is read concurrently). Discarding
    /// that count and trusting a single call to have read everything risks silent
    /// truncation, so the generated code loops, re-requesting the remaining amount from the
    /// new offset, until the full length is consumed. A call that makes zero progress
    /// (returns 0 while data is still outstanding) is treated as a hard error rather than
    /// spinning forever.
    fn emit_row_get_lob(col: &ResolvedColumn, index: usize, indent: &str, kind: LobKind) -> String {
        let name = &col.field_name;
        let locator_ty = kind.locator_ty();
        let buf_init = kind.buf_init();
        let unit = kind.unit();

        if col.nullable {
            // Inside the `Some(lob) => { ... }` arm, the locator is always bound to `lob`.
            let open_line = if kind.needs_open() {
                format!("{indent}        lob.open_file().await?;\n")
            } else {
                String::new()
            };
            format!(
                "{indent}let {name}: {ty} = match row.get::<Option<{locator_ty}<'_>>, _>({i})? {{\n\
                 {indent}    Some(lob) => {{\n\
                 {open_line}\
                 {indent}        let len = lob.len().await?;\n\
                 {indent}        let mut buf = {buf_init};\n\
                 {indent}        let mut read = 0usize;\n\
                 {indent}        while read < len {{\n\
                 {indent}            let n = lob.read(read, len - read, &mut buf).await?;\n\
                 {indent}            if n == 0 {{\n\
                 {indent}                return Err(sibyl::Error::Interface(format!(\"incomplete LOB read for column '{name}': expected {{}} {unit}, got {{}}\", len, read)));\n\
                 {indent}            }}\n\
                 {indent}            read += n;\n\
                 {indent}        }}\n\
                 {indent}        Some(buf)\n\
                 {indent}    }}\n\
                 {indent}    None => None,\n\
                 {indent}}};",
                indent = indent,
                open_line = open_line,
                name = name,
                i = index,
                ty = col.full_type,
                locator_ty = locator_ty,
                buf_init = buf_init,
                unit = unit,
            )
        } else {
            let open_line = if kind.needs_open() {
                format!("{indent}{name}_lob.open_file().await?;\n")
            } else {
                String::new()
            };
            format!(
                "{indent}let {name}_lob: {locator_ty}<'_> = row.get({i})?;\n\
                 {open_line}\
                 {indent}let {name}_len = {name}_lob.len().await?;\n\
                 {indent}let mut {name}: {ty} = {buf_init};\n\
                 {indent}let mut {name}_read = 0usize;\n\
                 {indent}while {name}_read < {name}_len {{\n\
                 {indent}    let {name}_n = {name}_lob.read({name}_read, {name}_len - {name}_read, &mut {name}).await?;\n\
                 {indent}    if {name}_n == 0 {{\n\
                 {indent}        return Err(sibyl::Error::Interface(format!(\"incomplete LOB read for column '{name}': expected {{}} {unit}, got {{}}\", {name}_len, {name}_read)));\n\
                 {indent}    }}\n\
                 {indent}    {name}_read += {name}_n;\n\
                 {indent}}}",
                indent = indent,
                open_line = open_line,
                name = name,
                i = index,
                ty = col.full_type,
                locator_ty = locator_ty,
                buf_init = buf_init,
                unit = unit,
            )
        }
    }
}

/// Distinguishes the three sibyl LOB locator shapes so `emit_row_get_lob` can emit the
/// right locator type, buffer type, and unit label. CLOB and NCLOB share sibyl's `CLOB<'_>`
/// type (NCLOB is just a CLOB with a different charset form, resolved internally by
/// sibyl's `charset_form()`), so they share `LobKind::Text`.
#[derive(Clone, Copy)]
enum LobKind {
    /// CLOB / NCLOB — character data, `CLOB<'_>` locator, `String` buffer.
    Text,
    /// BLOB — binary data, `BLOB<'_>` locator, `Vec<u8>` buffer.
    Blob,
    /// BFILE — binary data via a read-only external-file locator; `BFile<'_>` locator,
    /// `Vec<u8>` buffer. Must be opened with `open_file()` before it can be read.
    BFile,
}

impl LobKind {
    fn locator_ty(self) -> &'static str {
        match self {
            LobKind::Text => "CLOB",
            LobKind::Blob => "BLOB",
            LobKind::BFile => "BFile",
        }
    }

    fn buf_init(self) -> &'static str {
        match self {
            LobKind::Text => "String::new()",
            LobKind::Blob | LobKind::BFile => "Vec::new()",
        }
    }

    fn unit(self) -> &'static str {
        match self {
            LobKind::Text => "characters",
            LobKind::Blob | LobKind::BFile => "bytes",
        }
    }

    /// BFILE locators must be opened with `open_file()` before they can be read;
    /// CLOB/NCLOB/BLOB locators don't need this.
    fn needs_open(self) -> bool {
        matches!(self, LobKind::BFile)
    }
}

/// Rewrite $1, $2, ... positional params to :PARAM_NAME named params for Oracle sibyl 0.7.
impl CodegenBackend for RustSibylBackend {
    fn name(&self) -> &str {
        "rust-sibyl"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["oracle"]
    }

    fn file_header(&self) -> String {
        "use sibyl::*;\n".to_string()
    }

    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "#[derive(Debug, Clone)]");
        let _ = writeln!(out, "pub struct {} {{", struct_name);
        for col in columns {
            let _ = writeln!(out, "    pub {}: {},", col.field_name, col.full_type);
        }
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_model_struct(&self, table_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let singular = singularize(table_name);
        let name = to_pascal_case(&singular);
        self.generate_row_struct(&name, columns)
    }

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let positional_sql = super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |n| format!(":{n}"),
        );
        let sql = Self::sql_with_named_params(&positional_sql, params);

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.borrowed_type))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let has_returning = sql.to_uppercase().contains("RETURNING");

        let mut out = String::new();

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(
                    out,
                    "pub async fn {}<'a>(session: &'a Session<'a>{}{}) -> sibyl::Result<Option<{}>> {{",
                    func_name, sep, param_list, struct_name
                );

                if has_returning {
                    let full_sql = Self::sql_with_returning(&sql, columns);
                    let _ = writeln!(out, "    let stmt = session.prepare(\"{}\").await?;", full_sql);
                    for col in columns {
                        let _ = writeln!(out, "{}", Self::emit_out_var_decl_typed(col));
                    }
                    let in_pairs: Vec<String> = params
                        .iter()
                        .map(|p| format!("(\"{}\", {})", Self::named_placeholder(&p.field_name), p.field_name))
                        .collect();
                    let out_pairs: Vec<String> = columns
                        .iter()
                        .map(|col| {
                            format!(
                                "(\"{}\", {})",
                                Self::out_named_placeholder(&col.field_name),
                                Self::out_var_ref_for_returning(col)
                            )
                        })
                        .collect();
                    let all_pairs: Vec<String> = in_pairs.into_iter().chain(out_pairs).collect();
                    let args_expr = if all_pairs.len() == 1 {
                        all_pairs[0].clone()
                    } else {
                        format!("({})", all_pairs.join(", "))
                    };
                    let _ = writeln!(out, "    stmt.execute({}).await?;", args_expr);
                    for col in columns {
                        let _ = writeln!(out, "{}", Self::emit_out_var_conversion(col));
                    }
                    let field_assigns: Vec<String> = columns
                        .iter()
                        .map(|c| format!("{}: {}", c.field_name, c.field_name))
                        .collect();
                    let _ = writeln!(out, "    Ok(Some({} {{ {} }}))", struct_name, field_assigns.join(", "));
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(out, "    let stmt = session.prepare(\"{}\").await?;", sql);
                    let args_expr = Self::build_in_args(params);
                    let _ = writeln!(out, "    let rows = stmt.query({}).await?;", args_expr);
                    let _ = writeln!(out, "    if let Some(row) = rows.next().await? {{");
                    for (i, col) in columns.iter().enumerate() {
                        let _ = writeln!(out, "{}", Self::emit_row_get(col, i, "        "));
                    }
                    let field_assigns: Vec<String> = columns
                        .iter()
                        .map(|c| format!("{}: {}", c.field_name, c.field_name))
                        .collect();
                    let _ = writeln!(
                        out,
                        "        Ok(Some({} {{ {} }}))",
                        struct_name,
                        field_assigns.join(", ")
                    );
                    let _ = writeln!(out, "    }} else {{");
                    let _ = writeln!(out, "        Ok(None)");
                    let _ = writeln!(out, "    }}");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "pub async fn {}<'a>(session: &'a Session<'a>{}{}) -> sibyl::Result<Vec<{}>> {{",
                    func_name, sep, param_list, struct_name
                );
                let _ = writeln!(out, "    let stmt = session.prepare(\"{}\").await?;", sql);
                let args_expr = Self::build_in_args(params);
                let _ = writeln!(out, "    let rows = stmt.query({}).await?;", args_expr);
                let _ = writeln!(out, "    let mut results = Vec::new();");
                let _ = writeln!(out, "    while let Some(row) = rows.next().await? {{");
                for (i, col) in columns.iter().enumerate() {
                    let _ = writeln!(out, "{}", Self::emit_row_get(col, i, "        "));
                }
                let field_assigns: Vec<String> = columns
                    .iter()
                    .map(|c| format!("{}: {}", c.field_name, c.field_name))
                    .collect();
                let _ = writeln!(
                    out,
                    "        results.push({} {{ {} }});",
                    struct_name,
                    field_assigns.join(", ")
                );
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "    Ok(results)");
                let _ = write!(out, "}}");
            }
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "pub async fn {}<'a>(session: &'a Session<'a>{}{}) -> sibyl::Result<()> {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(out, "    let stmt = session.prepare(\"{}\").await?;", sql);
                let args_expr = Self::build_in_args(params);
                let _ = writeln!(out, "    stmt.execute({}).await?;", args_expr);
                let _ = writeln!(out, "    Ok(())");
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "pub async fn {}<'a>(session: &'a Session<'a>{}{}) -> sibyl::Result<usize> {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(out, "    let stmt = session.prepare(\"{}\").await?;", sql);
                let args_expr = Self::build_in_args(params);
                let _ = writeln!(out, "    let num_rows = stmt.execute({}).await?;", args_expr);
                let _ = writeln!(out, "    Ok(num_rows)");
                let _ = write!(out, "}}");
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}_batch", func_name);
                let _ = writeln!(
                    out,
                    "pub async fn {}<'a>(session: &'a Session<'a>, items: &[({})]) -> sibyl::Result<()> {{",
                    batch_fn_name,
                    params
                        .iter()
                        .map(|p| p.full_type.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                let _ = writeln!(out, "    let stmt = session.prepare(\"{}\").await?;", sql);
                let _ = writeln!(out, "    for item in items {{");
                let item_pairs: Vec<String> = params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| format!("(\"{}\", &item.{})", Self::named_placeholder(&p.field_name), i))
                    .collect();
                let args_expr = if item_pairs.len() == 1 {
                    item_pairs[0].clone()
                } else {
                    format!("({})", item_pairs.join(", "))
                };
                let _ = writeln!(out, "        stmt.execute({}).await?;", args_expr);
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "    Ok(())");
                let _ = write!(out, "}}");
            }
            QueryCommand::Grouped => unreachable!("grouped queries are routed to generate_grouped_query_fn"),
        }

        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "#[derive(Debug, Clone, PartialEq)]");
        let _ = writeln!(out, "pub enum {} {{", type_name);
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "    {},", variant);
        }
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
        let _ = writeln!(out, "impl {} {{", type_name);
        let _ = writeln!(out, "    pub fn as_str(&self) -> &'static str {{");
        let _ = writeln!(out, "        match self {{");
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "            {}::{} => \"{}\",", type_name, variant, value);
        }
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "    }}");
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

        let _ = writeln!(out, "#[derive(Debug, Clone)]");
        let _ = writeln!(out, "pub struct {} {{", child_struct_name);
        for col in child_columns {
            let _ = writeln!(out, "    pub {}: {},", col.field_name, col.full_type);
        }
        let _ = writeln!(out, "}}");

        let _ = writeln!(out);

        let _ = writeln!(out, "#[derive(Debug, Clone)]");
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
        let child_columns = request.child_columns;
        let params = request.params;
        let key_column = request.key_column;

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let key_field = to_snake_case(key_column);
        let mut out = String::new();

        let positional_sql = super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |n| format!(":{n}"),
        );
        let sql = Self::sql_with_named_params(&positional_sql, params);

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.borrowed_type))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let args_expr = Self::build_in_args(params);

        let _ = writeln!(
            out,
            "pub async fn {}<'a>(session: &'a Session<'a>{}{}) -> sibyl::Result<Vec<{}>> {{",
            func_name, sep, param_list, parent_struct_name
        );

        let _ = writeln!(out, "    let stmt = session.prepare(\"{}\").await?;", sql);
        let _ = writeln!(out, "    let rows = stmt.query({}).await?;", args_expr);
        let _ = writeln!(out, "    let mut result: Vec<{}> = Vec::new();", parent_struct_name);
        let _ = writeln!(out, "    while let Some(row) = rows.next().await? {{");

        for (i, col) in parent_columns.iter().enumerate() {
            let _ = writeln!(out, "{}", Self::emit_row_get(col, i, "        "));
        }

        let _ = writeln!(out, "        let key = {key_field}.clone();");

        let parent_len = parent_columns.len();
        for (j, col) in child_columns.iter().enumerate() {
            let _ = writeln!(out, "{}", Self::emit_row_get(col, parent_len + j, "        "));
        }

        let _ = writeln!(out, "        let child = {} {{", child_struct_name);
        for col in child_columns {
            let _ = writeln!(out, "            {},", col.field_name);
        }
        let _ = writeln!(out, "        }};");

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
        let name = to_pascal_case(&composite.sql_name);
        let mut out = String::new();
        let _ = writeln!(out, "#[derive(Debug, Clone)]");
        let _ = writeln!(out, "pub struct {} {{", name);
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
}

#[cfg(test)]
mod tests {
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, GroupByConfig};
    use scythe_core::parser::QueryCommand;

    use super::RustSibylBackend;
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
    fn test_grouped_sibyl_structs() {
        let backend = RustSibylBackend::new("oracle").unwrap();
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

        assert!(result.model_struct.is_none(), "grouped must not produce a model_struct");
    }

    #[test]
    fn test_grouped_sibyl_query_fn() {
        let backend = RustSibylBackend::new("oracle").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("pub async fn get_users_with_orders<'a>(session: &'a Session<'a>)"),
            "missing fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("-> sibyl::Result<Vec<GetUsersWithOrdersRow>>"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("rows.next().await?"),
            "fn must iterate rows; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersChildRow {"),
            "fn must construct child struct; got:\n{query_fn}"
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

    /// Regression test for the "Interface(\"cannot return as a String\")" runtime error:
    /// sibyl's `FromSql<String>` only matches `ColumnBuffer::Text` (VARCHAR2/CHAR); an
    /// Oracle CLOB column's buffer is `ColumnBuffer::CLOB`, so `row.get::<String, _>(i)`
    /// fails at runtime. CLOB columns must be fetched as a `CLOB<'_>` locator and read
    /// explicitly instead.
    #[test]
    fn test_clob_column_reads_via_lob_locator_not_row_get_string() {
        let backend = RustSibylBackend::new("oracle").unwrap();
        let query = AnalyzedQuery::build(|aq| {
            aq.name = "GetOrdersByUser".to_string();
            aq.command = QueryCommand::Many;
            aq.sql = "SELECT id, notes FROM orders".to_string();
            aq.columns = vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    sql_type: "integer".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "notes".to_string(),
                    sql_type: "clob".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: true,
                    ..Default::default()
                },
            ];
        });

        let result = generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("row.get::<Option<CLOB<'_>>, _>(1)?"),
            "CLOB column must be fetched as a LOB locator, not a String; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("lob.read(read, len - read, &mut buf).await?"),
            "CLOB content must be read explicitly via LOB::read; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("let notes: Option<String> = row.get(1)?;"),
            "CLOB column must not be read via the plain String path; got:\n{query_fn}"
        );
    }

    /// Regression test: a discarded `LOB::read` return count risks silent truncation for
    /// large LOBs (sibyl's own doc example shows `read()` can return fewer chars/bytes than
    /// requested). Generated code must loop until the full reported length has been read,
    /// and must error out — not spin forever — if a read call makes zero progress.
    #[test]
    fn test_clob_read_loops_until_full_length_consumed() {
        let backend = RustSibylBackend::new("oracle").unwrap();
        let query = AnalyzedQuery::build(|aq| {
            aq.name = "GetOrdersByUser".to_string();
            aq.command = QueryCommand::Many;
            aq.sql = "SELECT notes FROM orders".to_string();
            aq.columns = vec![AnalyzedColumn {
                name: "notes".to_string(),
                sql_type: "clob".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                ..Default::default()
            }];
        });

        let result = generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("let mut notes_read = 0usize;"),
            "must track accumulated read progress; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("while notes_read < notes_len {"),
            "must loop until the full LOB length is consumed; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("let notes_n = notes_lob.read(notes_read, notes_len - notes_read, &mut notes).await?;"),
            "each iteration must request only the remaining amount from the new offset; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("if notes_n == 0 {") && query_fn.contains("return Err(sibyl::Error::Interface("),
            "a zero-progress read must be treated as a hard error, not silently accepted or looped forever; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("notes_read += notes_n;"),
            "progress must accumulate across iterations; got:\n{query_fn}"
        );
    }

    /// BLOB columns must take the same LOB-locator path as CLOB (the doc comment on
    /// `emit_row_get` has long promised this), but must decode into `Vec<u8>` — BLOB is
    /// binary data, not text, and forcing it through `String` would corrupt non-UTF8 bytes.
    #[test]
    fn test_blob_column_reads_via_lob_locator_as_bytes() {
        let backend = RustSibylBackend::new("oracle").unwrap();
        let query = AnalyzedQuery::build(|aq| {
            aq.name = "GetAttachment".to_string();
            aq.command = QueryCommand::Opt;
            aq.sql = "SELECT payload FROM attachments".to_string();
            aq.columns = vec![AnalyzedColumn {
                name: "payload".to_string(),
                sql_type: "blob".to_string(),
                neutral_type: "bytes".to_string(),
                nullable: false,
                ..Default::default()
            }];
        });

        let result = generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            query_fn.contains("let payload_lob: BLOB<'_> = row.get(0)?;"),
            "BLOB column must be fetched as a LOB locator; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("let mut payload: Vec<u8> = Vec::new();"),
            "BLOB content must decode into Vec<u8>, not String; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("payload_lob.read(payload_read, payload_len - payload_read, &mut payload).await?"),
            "BLOB content must be read explicitly, looping until fully consumed; got:\n{query_fn}"
        );
        assert!(
            row_struct.contains("pub payload: Vec<u8>,"),
            "row struct field must be Vec<u8> for a BLOB column; got:\n{row_struct}"
        );
    }

    /// NCLOB shares sibyl's `CLOB<'_>` locator type (distinguished internally by charset
    /// form), so it must take the same text LOB path as CLOB — not the broken String path.
    #[test]
    fn test_nclob_column_reads_via_lob_locator() {
        let backend = RustSibylBackend::new("oracle").unwrap();
        let query = AnalyzedQuery::build(|aq| {
            aq.name = "GetDoc".to_string();
            aq.command = QueryCommand::Opt;
            aq.sql = "SELECT body FROM docs".to_string();
            aq.columns = vec![AnalyzedColumn {
                name: "body".to_string(),
                sql_type: "nclob".to_string(),
                neutral_type: "string".to_string(),
                nullable: true,
                ..Default::default()
            }];
        });

        let result = generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("row.get::<Option<CLOB<'_>>, _>(0)?"),
            "NCLOB column must be fetched via the CLOB locator type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Some(buf)"),
            "NCLOB column must produce Option<String> content; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("let body: Option<String> = row.get(0)?;"),
            "NCLOB column must not be read via the plain String path; got:\n{query_fn}"
        );
    }

    /// BFILE locators must be opened with `open_file()` before they can be read (sibyl
    /// requires this — a fresh BFILE locator is not implicitly open), and read as bytes via
    /// the `BFile<'_>` locator type, not the plain String path.
    #[test]
    fn test_bfile_column_opens_before_reading_as_bytes() {
        let backend = RustSibylBackend::new("oracle").unwrap();
        let query = AnalyzedQuery::build(|aq| {
            aq.name = "GetExternalDoc".to_string();
            aq.command = QueryCommand::Opt;
            aq.sql = "SELECT contents FROM external_docs".to_string();
            aq.columns = vec![AnalyzedColumn {
                name: "contents".to_string(),
                sql_type: "bfile".to_string(),
                neutral_type: "bytes".to_string(),
                nullable: false,
                ..Default::default()
            }];
        });

        let result = generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("let contents_lob: BFile<'_> = row.get(0)?;"),
            "BFILE column must be fetched as a BFile locator; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("contents_lob.open_file().await?;"),
            "BFILE locator must be opened before reading; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("let mut contents: Vec<u8> = Vec::new();"),
            "BFILE content must decode into Vec<u8>; got:\n{query_fn}"
        );
        let open_pos = query_fn.find("open_file().await?").unwrap();
        let read_pos = query_fn.find("contents_lob.read(").unwrap();
        assert!(
            open_pos < read_pos,
            "open_file() must happen before read(); got:\n{query_fn}"
        );
    }

    #[test]
    fn test_non_clob_string_column_still_uses_plain_row_get() {
        let backend = RustSibylBackend::new("oracle").unwrap();
        let query = AnalyzedQuery::build(|aq| {
            aq.name = "GetUserById".to_string();
            aq.command = QueryCommand::Opt;
            aq.sql = "SELECT id, name FROM users".to_string();
            aq.columns = vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    sql_type: "integer".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "name".to_string(),
                    sql_type: "varchar(255)".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    ..Default::default()
                },
            ];
        });

        let result = generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("let name: String = row.get(1)?;"),
            "VARCHAR2 columns must keep using plain row.get(); got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("CLOB"),
            "non-CLOB query must not reference CLOB; got:\n{query_fn}"
        );
    }

    /// #103: rust-sibyl takes no `[[sql.gen]]` options at all and does not
    /// override `apply_options`, so it exercises the `CodegenBackend` trait
    /// default directly. Before this, that default was `Ok(())` for any map,
    /// so any key -- typo or otherwise -- was silently discarded. The
    /// default now rejects every key, which is the correct behavior for a
    /// backend with nothing to configure.
    #[test]
    fn apply_options_rejects_any_key_via_the_trait_default() {
        use crate::backend_trait::CodegenBackend;

        let mut backend = RustSibylBackend::new("oracle").unwrap();
        let err = backend
            .apply_options(&std::collections::HashMap::from([(
                "row_type".to_string(),
                "pydantic".to_string(),
            )]))
            .expect_err("rust-sibyl takes no options, so any key must be rejected");
        assert_eq!(err.code, scythe_core::errors::ErrorCode::InvalidConfig);
        assert!(err.message.contains("row_type"), "{}", err.message);
    }
}
