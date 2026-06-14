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
                    ErrorCode::InternalError,
                    format!("rust-sibyl only supports Oracle, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::load_or_default_manifest(
            "backends/rust-sibyl/manifest.toml",
            DEFAULT_MANIFEST_TOML,
        )?;
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
                    .map(|p| {
                        format!(
                            "(\"{}\", {})",
                            Self::named_placeholder(&p.field_name),
                            p.field_name
                        )
                    })
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
        // Replace in reverse order so :10 is replaced before :1.
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
        // Remove any existing RETURNING clause — we'll rebuild it with the INTO.
        // The base SQL may already contain "RETURNING col1, col2" without "INTO".
        let upper = base_sql.to_uppercase();
        if let Some(ret_pos) = upper.find("RETURNING") {
            let prefix = &base_sql[..ret_pos];
            // Extract column list between RETURNING and end (or INTO)
            let rest = &base_sql[ret_pos + "RETURNING".len()..];
            let col_list = if let Some(into_pos) = rest.to_uppercase().find(" INTO ") {
                rest[..into_pos].trim()
            } else {
                rest.trim()
            };
            format!(
                "{}RETURNING {} INTO {}",
                prefix,
                col_list,
                out_names.join(", ")
            )
        } else {
            // No RETURNING in SQL — shouldn't happen, but handle gracefully
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
                // Use sibyl Date to receive the value, then convert to chrono.
                format!("    let mut out_{} = Date::new(session);", col.field_name)
            }
            _ => {
                // Both nullable and non-nullable strings use `String` as bind buffer.
                // Nullable case: checked via stmt.is_null() after execute.
                format!("    let mut out_{} = String::new();", col.field_name)
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
            "int16" => format!(
                "    let {} = out_{} as i16;",
                col.field_name, col.field_name
            ),
            "int32" => format!(
                "    let {} = out_{} as i32;",
                col.field_name, col.field_name
            ),
            "int64" => format!("    let {} = out_{};", col.field_name, col.field_name),
            "float32" => format!(
                "    let {} = out_{} as f32;",
                col.field_name, col.field_name
            ),
            "float64" | "decimal" => {
                format!("    let {} = out_{};", col.field_name, col.field_name)
            }
            "date" | "datetime" | "datetime_tz" => {
                // Convert sibyl Date to chrono::NaiveDateTime via date_and_time().
                format!(
                    "    let {name} = {{ let (y, mo, d, h, mi, s) = out_{name}.date_and_time(); chrono::NaiveDate::from_ymd_opt(y as i32, mo as u32, d as u32).and_then(|dt| dt.and_hms_opt(h as u32, mi as u32, s as u32)).expect(\"invalid date from Oracle\") }};",
                    name = col.field_name
                )
            }
            _ => {
                if col.nullable {
                    // Nullable string: check is_null() on the named param to produce Option.
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
    fn emit_row_get(col: &ResolvedColumn, index: usize, indent: &str) -> String {
        match col.neutral_type.as_str() {
            "date" | "datetime" | "datetime_tz" => {
                // Oracle DATE → sibyl Date<'_>, then chrono.
                // For nullable dates: get Option<Date<'_>> then convert the inner value.
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
                // Integers, floats, strings — direct get with LHS type annotation.
                // Use full_type (which includes Option<> wrapper for nullable columns).
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
}

/// Rewrite $1, $2, ... positional params to :PARAM_NAME named params for Oracle sibyl 0.7.
impl CodegenBackend for RustSibylBackend {
    fn name(&self) -> &str {
        "rust-sibyl"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["oracle"]
    }

    fn file_header(&self) -> String {
        "// Auto-generated by scythe. Do not edit.\n\
         use sibyl::*;\n"
            .to_string()
    }

    fn generate_row_struct(
        &self,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
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

    fn generate_model_struct(
        &self,
        table_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
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
        // Rewrite $N placeholders to :N (Oracle positional), then rewrite to named :PARAM_NAME.
        let positional_sql = super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(
                &analyzed.sql,
                &analyzed.optional_params,
                &analyzed.params,
            ),
            |n| format!(":{n}"),
        );
        let sql = Self::sql_with_named_params(&positional_sql, params);

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.borrowed_type))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        // Check if this is a DML with RETURNING (INSERT/UPDATE/DELETE RETURNING)
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
                    // sibyl 0.7 RETURNING: SQL has RETURNING cols INTO :OUT_COL_NAME ...
                    // execute() args tuple includes `(":OUT_COL_NAME", &mut out_col)`.
                    let full_sql = Self::sql_with_returning(&sql, columns);
                    let _ = writeln!(
                        out,
                        "    let stmt = session.prepare(\"{}\").await?;",
                        full_sql
                    );
                    // Declare out variables.
                    for col in columns {
                        let _ = writeln!(out, "{}", Self::emit_out_var_decl_typed(col));
                    }
                    // Build execute args combining IN params and &mut OUT vars.
                    let in_pairs: Vec<String> = params
                        .iter()
                        .map(|p| {
                            format!(
                                "(\"{}\", {})",
                                Self::named_placeholder(&p.field_name),
                                p.field_name
                            )
                        })
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
                    // Convert out vars to target types.
                    for col in columns {
                        let _ = writeln!(out, "{}", Self::emit_out_var_conversion(col));
                    }
                    let field_assigns: Vec<String> = columns
                        .iter()
                        .map(|c| format!("{}: {}", c.field_name, c.field_name))
                        .collect();
                    let _ = writeln!(
                        out,
                        "    Ok(Some({} {{ {} }}))",
                        struct_name,
                        field_assigns.join(", ")
                    );
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
                let _ = writeln!(
                    out,
                    "    let num_rows = stmt.execute({}).await?;",
                    args_expr
                );
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
                    .map(|(i, p)| {
                        format!(
                            "(\"{}\", &item.{})",
                            Self::named_placeholder(&p.field_name),
                            i
                        )
                    })
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
            QueryCommand::Grouped => unreachable!("Grouped is rewritten to Many before codegen"),
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
            let _ = writeln!(
                out,
                "            {}::{} => \"{}\",",
                type_name, variant, value
            );
        }
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "    }}");
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
                    ScytheError::new(
                        ErrorCode::InternalError,
                        format!("composite field type error: {}", e),
                    )
                })?;
            let _ = writeln!(
                out,
                "    pub {}: {},",
                to_snake_case(&field.name),
                rust_type
            );
        }
        let _ = write!(out, "}}");
        Ok(out)
    }
}
