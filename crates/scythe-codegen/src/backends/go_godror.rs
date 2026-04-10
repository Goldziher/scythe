use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case,
};
use scythe_backend::types::resolve_type;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/go-godror.toml");

pub struct GoGodrorBackend {
    manifest: BackendManifest,
}

impl GoGodrorBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "oracle" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("go-godror only supports Oracle, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::load_or_default_manifest(
            "backends/go-godror/manifest.toml",
            DEFAULT_MANIFEST_TOML,
        )?;
        Ok(Self { manifest })
    }
}

/// Rewrite $1, $2, ... to :1, :2, ... for Oracle.
impl CodegenBackend for GoGodrorBackend {
    fn name(&self) -> &str {
        "go-godror"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["oracle"]
    }

    fn file_header(&self) -> String {
        "package queries\n\nimport (\n\t\"context\"\n\t\"database/sql\"\n)\n".to_string()
    }

    fn generate_row_struct(
        &self,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "type {} struct {{", struct_name);
        for col in columns {
            let field = to_pascal_case(&col.field_name);
            let json_tag = &col.field_name;
            let _ = writeln!(out, "\t{} {} `json:\"{}\"`", field, col.full_type, json_tag);
        }
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_model_struct(
        &self,
        table_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let name = to_pascal_case(table_name);
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
        let sql = super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(
                &analyzed.sql,
                &analyzed.optional_params,
                &analyzed.params,
            ),
            |n| format!(":{n}"),
        );

        let param_list = params
            .iter()
            .map(|p| format!("{} {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let args = if params.is_empty() {
            String::new()
        } else {
            format!(
                ", {}",
                params
                    .iter()
                    .map(|p| p.field_name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let mut out = String::new();

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *sql.DB{}{}) (*{}, error) {{",
                    func_name, sep, param_list, struct_name
                );
                let _ = writeln!(out, "\trow := db.QueryRowContext(ctx, \"{}\"{})", sql, args);
                let _ = writeln!(out, "\tvar item {}", struct_name);
                let scan_fields: Vec<String> = columns
                    .iter()
                    .map(|c| format!("&item.{}", to_pascal_case(&c.field_name)))
                    .collect();
                let _ = writeln!(
                    out,
                    "\tif err := row.Scan({}); err != nil {{",
                    scan_fields.join(", ")
                );
                let _ = writeln!(out, "\t\tif err == sql.ErrNoRows {{");
                let _ = writeln!(out, "\t\t\treturn nil, nil");
                let _ = writeln!(out, "\t\t}}");
                let _ = writeln!(out, "\t\treturn nil, err");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\treturn &item, nil");
                let _ = write!(out, "}}");
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *sql.DB{}{}) ([]{}, error) {{",
                    func_name, sep, param_list, struct_name
                );
                let _ = writeln!(
                    out,
                    "\trows, err := db.QueryContext(ctx, \"{}\"{})",
                    sql, args
                );
                let _ = writeln!(out, "\tif err != nil {{");
                let _ = writeln!(out, "\t\treturn nil, err");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\tdefer rows.Close()");
                let _ = writeln!(out, "\tvar items []{}", struct_name);
                let _ = writeln!(out, "\tfor rows.Next() {{");
                let _ = writeln!(out, "\t\tvar item {}", struct_name);
                let scan_fields: Vec<String> = columns
                    .iter()
                    .map(|c| format!("&item.{}", to_pascal_case(&c.field_name)))
                    .collect();
                let _ = writeln!(
                    out,
                    "\t\tif err := rows.Scan({}); err != nil {{",
                    scan_fields.join(", ")
                );
                let _ = writeln!(out, "\t\t\treturn nil, err");
                let _ = writeln!(out, "\t\t}}");
                let _ = writeln!(out, "\t\titems = append(items, item)");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\treturn items, rows.Err()");
                let _ = write!(out, "}}");
            }
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *sql.DB{}{}) error {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(out, "\t_, err := db.ExecContext(ctx, \"{}\"{})", sql, args);
                let _ = writeln!(out, "\treturn err");
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *sql.DB{}{}) (int64, error) {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(
                    out,
                    "\tresult, err := db.ExecContext(ctx, \"{}\"{})",
                    sql, args
                );
                let _ = writeln!(out, "\tif err != nil {{");
                let _ = writeln!(out, "\t\treturn 0, err");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\treturn result.RowsAffected()");
                let _ = write!(out, "}}");
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}Batch", func_name);
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *sql.DB, items [][]any) error {{",
                    batch_fn_name
                );
                let _ = writeln!(out, "\ttx, err := db.BeginTx(ctx, nil)");
                let _ = writeln!(out, "\tif err != nil {{");
                let _ = writeln!(out, "\t\treturn err");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\tstmt, err := tx.PrepareContext(ctx, \"{}\")", sql);
                let _ = writeln!(out, "\tif err != nil {{");
                let _ = writeln!(out, "\t\t_ = tx.Rollback()");
                let _ = writeln!(out, "\t\treturn err");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\tdefer stmt.Close()");
                let _ = writeln!(out, "\tfor _, item := range items {{");
                let _ = writeln!(
                    out,
                    "\t\tif _, err := stmt.ExecContext(ctx, item...); err != nil {{"
                );
                let _ = writeln!(out, "\t\t\t_ = tx.Rollback()");
                let _ = writeln!(out, "\t\t\treturn err");
                let _ = writeln!(out, "\t\t}}");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\treturn tx.Commit()");
                let _ = write!(out, "}}");
            }
            QueryCommand::Grouped => unreachable!("Grouped is rewritten to Many before codegen"),
        }

        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "type {} string", type_name);
        let _ = writeln!(out);
        let _ = writeln!(out, "const (");
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(
                out,
                "\t{}{} {} = \"{}\"",
                type_name, variant, type_name, value
            );
        }
        let _ = write!(out, ")");
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = to_pascal_case(&composite.sql_name);
        let mut out = String::new();
        let _ = writeln!(out, "type {} struct {{", name);
        for field in &composite.fields {
            let go_type = resolve_type(&field.neutral_type, &self.manifest, false)
                .map(|t| t.into_owned())
                .unwrap_or_else(|_| "any".to_string());
            let _ = writeln!(out, "\t{} {}", to_pascal_case(&field.name), go_type);
        }
        let _ = write!(out, "}}");
        Ok(out)
    }
}
