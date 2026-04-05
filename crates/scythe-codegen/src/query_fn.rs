use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    enum_type_name, fn_name, row_struct_name, to_pascal_case, to_snake_case,
};

use scythe_core::analyzer::AnalyzedQuery;
use scythe_core::errors::ScytheError;
use scythe_core::parser::QueryCommand;

use super::enum_gen::rewrite_sql_for_enums;
use super::resolve_param_type;
use super::structs::singularize;

/// Strip SQL comments, trailing semicolons, and excess whitespace.
fn clean_sql(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string()
}

pub(super) fn generate_query_fn(
    analyzed: &AnalyzedQuery,
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    let func_name = fn_name(&analyzed.name, &manifest.naming);
    let struct_name = if let Some(ref table_name) = analyzed.source_table {
        let singular = singularize(table_name);
        to_pascal_case(&singular).into_owned()
    } else {
        row_struct_name(&analyzed.name, &manifest.naming)
    };

    let mut out = String::new();

    // Deprecated annotation
    if let Some(ref msg) = analyzed.deprecated {
        let _ = writeln!(out, "#[deprecated(note = \"{}\")]", msg);
    }

    // Build parameter list
    let mut param_parts: Vec<String> = vec!["pool: &sqlx::PgPool".to_string()];
    for param in &analyzed.params {
        let param_name = to_snake_case(&param.name);
        let rust_type = resolve_param_type(param, manifest)?;
        let rust_type = param_type_to_borrowed(&rust_type);
        param_parts.push(format!("{}: {}", param_name, rust_type));
    }

    // Return type
    let return_type = match &analyzed.command {
        QueryCommand::One => struct_name.clone(),
        QueryCommand::Many => format!("Vec<{}>", struct_name),
        QueryCommand::Exec => "()".to_string(),
        QueryCommand::ExecResult => "sqlx::postgres::PgQueryResult".to_string(),
        QueryCommand::ExecRows => "u64".to_string(),
        QueryCommand::Batch => format!("Vec<{}>", struct_name),
    };

    // Function signature - all params on one line
    let _ = writeln!(
        out,
        "pub async fn {}({}) -> Result<{}, sqlx::Error> {{",
        func_name,
        param_parts.join(", "),
        return_type
    );

    // Clean SQL: strip comments, trailing semicolons, whitespace
    let sql_raw = clean_sql(&analyzed.sql);

    // Rewrite SQL for enum columns: add "column: EnumType" aliases for sqlx
    let sql = rewrite_sql_for_enums(&sql_raw, &analyzed.columns, manifest);

    // Query body
    let has_row_struct = matches!(
        analyzed.command,
        QueryCommand::One | QueryCommand::Many | QueryCommand::Batch
    );

    // Build bind params string
    let bind_params: String = analyzed
        .params
        .iter()
        .map(|p| {
            let param_name = to_snake_case(&p.name);
            if p.neutral_type.starts_with("enum::") {
                let enum_name = p.neutral_type.strip_prefix("enum::").unwrap();
                let rust_type = enum_type_name(enum_name, &manifest.naming);
                format!(", {} as &{}", param_name, rust_type)
            } else {
                format!(", {}", param_name)
            }
        })
        .collect();

    let is_exec_rows = matches!(analyzed.command, QueryCommand::ExecRows);

    if is_exec_rows {
        // ExecRows: let result = sqlx::query!(...) pattern
        if has_row_struct && !analyzed.columns.is_empty() {
            let _ = write!(
                out,
                "    let result = sqlx::query_as!({}, \"{}\"{})",
                struct_name, sql, bind_params
            );
        } else {
            let _ = write!(
                out,
                "    let result = sqlx::query!(\"{}\"{})",
                sql, bind_params
            );
        }
    } else if has_row_struct && !analyzed.columns.is_empty() {
        let _ = write!(
            out,
            "    sqlx::query_as!({}, \"{}\"{})",
            struct_name, sql, bind_params
        );
    } else {
        let _ = write!(out, "    sqlx::query!(\"{}\"{})", sql, bind_params);
    }

    let _ = writeln!(out);

    // Fetch method
    let fetch_method = match &analyzed.command {
        QueryCommand::One => ".fetch_one(pool)",
        QueryCommand::Many => ".fetch_all(pool)",
        QueryCommand::Exec => ".execute(pool)",
        QueryCommand::ExecResult => ".execute(pool)",
        QueryCommand::ExecRows => ".execute(pool)",
        QueryCommand::Batch => ".fetch_all(pool)",
    };

    let _ = write!(out, "        {}", fetch_method);
    let _ = writeln!(out);

    // Post-processing for exec variants
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

/// Convert a resolved Rust type to its borrowed form for function parameters.
/// Copy types (primitives) stay as-is; String becomes &str; other non-Copy types get a & prefix.
pub(super) fn param_type_to_borrowed(rust_type: &str) -> String {
    // Copy types that should stay owned in function params
    let copy_types = ["bool", "i16", "i32", "i64", "f32", "f64", "u64"];
    if copy_types.contains(&rust_type) {
        return rust_type.to_string();
    }
    // String -> &str
    if rust_type == "String" {
        return "&str".to_string();
    }
    // Option<T> wrapping: Option<String> -> Option<&str>, Option<Copy> stays, Option<Other> -> Option<&Other>
    if let Some(inner) = rust_type
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
    {
        let borrowed_inner = param_type_to_borrowed(inner);
        return format!("Option<{}>", borrowed_inner);
    }
    // Vec<T> -> &[T] (slice reference)
    if let Some(inner) = rust_type
        .strip_prefix("Vec<")
        .and_then(|s| s.strip_suffix('>'))
    {
        return format!("&[{}]", inner);
    }
    // Everything else gets a & prefix
    format!("&{}", rust_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_param_type_to_borrowed_string() {
        assert_eq!(param_type_to_borrowed("String"), "&str");
    }

    #[test]
    fn test_param_type_to_borrowed_vec() {
        assert_eq!(param_type_to_borrowed("Vec<i32>"), "&[i32]");
        assert_eq!(param_type_to_borrowed("Vec<String>"), "&[String]");
    }

    #[test]
    fn test_param_type_to_borrowed_passthrough() {
        assert_eq!(param_type_to_borrowed("i32"), "i32");
        assert_eq!(param_type_to_borrowed("i64"), "i64");
        assert_eq!(param_type_to_borrowed("bool"), "bool");
        assert_eq!(param_type_to_borrowed("f64"), "f64");
    }

    #[test]
    fn test_param_type_to_borrowed_option_string() {
        assert_eq!(param_type_to_borrowed("Option<String>"), "Option<&str>");
    }

    #[test]
    fn test_param_type_to_borrowed_option_copy() {
        assert_eq!(param_type_to_borrowed("Option<i32>"), "Option<i32>");
    }

    #[test]
    fn test_param_type_to_borrowed_other() {
        assert_eq!(param_type_to_borrowed("Uuid"), "&Uuid");
        assert_eq!(param_type_to_borrowed("NaiveDateTime"), "&NaiveDateTime");
    }
}
