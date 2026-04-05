use std::fmt::Write;

use ahash::AHashSet;
use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::enum_type_name;

use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery};
use scythe_core::errors::ScytheError;

pub(super) fn generate_enum_defs(
    analyzed: &AnalyzedQuery,
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    use scythe_backend::naming::enum_variant_name;

    let mut out = String::new();
    let mut seen_enums: AHashSet<String> = AHashSet::new();

    // Collect enum types from columns and params
    let enum_sources: Vec<&str> = analyzed
        .columns
        .iter()
        .filter_map(|col| col.neutral_type.strip_prefix("enum::"))
        .chain(
            analyzed
                .params
                .iter()
                .filter_map(|p| p.neutral_type.strip_prefix("enum::")),
        )
        .collect();

    for sql_name in enum_sources {
        if !seen_enums.insert(sql_name.to_string()) {
            continue;
        }

        let type_name = enum_type_name(sql_name, &manifest.naming);

        if !out.is_empty() {
            let _ = writeln!(out);
        }

        let _ = writeln!(out, "#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]");
        let _ = writeln!(
            out,
            "#[sqlx(type_name = \"{}\", rename_all = \"snake_case\")]",
            sql_name
        );
        let _ = writeln!(out, "pub enum {} {{", type_name);

        // Use actual enum values from the analyzed query
        if let Some(enum_info) = analyzed.enums.iter().find(|e| e.sql_name == sql_name) {
            for value in &enum_info.values {
                let variant = enum_variant_name(value, &manifest.naming);
                let _ = writeln!(out, "    {},", variant);
            }
        }

        let _ = write!(out, "}}");
    }

    Ok(out)
}

/// Generate a single enum definition from an EnumInfo.
pub fn generate_single_enum_def(
    enum_info: &scythe_core::analyzer::EnumInfo,
    manifest: &BackendManifest,
) -> String {
    use scythe_backend::naming::enum_variant_name;

    let mut out = String::with_capacity(256);
    let type_name = enum_type_name(&enum_info.sql_name, &manifest.naming);

    let _ = writeln!(out, "#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]");
    let _ = writeln!(
        out,
        "#[sqlx(type_name = \"{}\", rename_all = \"snake_case\")]",
        enum_info.sql_name
    );
    let _ = writeln!(out, "pub enum {type_name} {{");

    for value in &enum_info.values {
        let variant = enum_variant_name(value, &manifest.naming);
        let _ = writeln!(out, "    {variant},");
    }

    let _ = write!(out, "}}");

    out
}

/// Rewrite SQL to add enum type annotations for sqlx.
/// For enum columns in SELECT, adds `column AS "column: EnumType"` aliases.
pub(super) fn rewrite_sql_for_enums(
    sql: &str,
    columns: &[AnalyzedColumn],
    manifest: &BackendManifest,
) -> String {
    // Find enum columns that need annotation
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
        // Look for bare column reference in SELECT list and add alias
        // Try to find and replace the column name with its annotated version
        // Handle both "column" and "table.column" patterns
        let alias = format!("{} AS \\\"{}: {}\\\"", col_name, col_name, annotation);
        // Simple word-boundary replacement in the SELECT portion
        // Find the SELECT ... FROM boundary
        if let Some(from_pos) = result.to_uppercase().find(" FROM ") {
            let select_part = &result[..from_pos];
            let rest = &result[from_pos..];

            // Replace bare column name (not already aliased)
            let new_select = replace_column_in_select(select_part, col_name, &alias);
            result = format!("{}{}", new_select, rest);
        }
    }
    result
}

/// Replace a bare column name in a SELECT clause with an aliased version.
pub(super) fn replace_column_in_select(select: &str, col_name: &str, replacement: &str) -> String {
    // Simple approach: find the column name as a standalone word
    let mut result = select.to_string();
    // Check for "column" as a whole word (preceded by comma/space/SELECT and followed by comma/space/FROM)
    let patterns = [format!(", {}", col_name), format!(" {}", col_name)];
    for pattern in &patterns {
        if let Some(pos) = result.rfind(pattern.as_str()) {
            let after = pos + pattern.len();
            // Check that the next char is comma, space, or end of string
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
