use std::fmt::Write;

use scythe_core::errors::{ErrorCode, ScytheError};

use crate::backend_trait::ResolvedColumn;

/// Supported TypeScript row type styles for generated code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TsRowType {
    #[default]
    Interface,
    Zod,
}

impl TsRowType {
    /// Parse a row_type option string into a `TsRowType`.
    pub fn from_option(value: &str) -> Result<Self, ScytheError> {
        match value {
            "interface" => Ok(Self::Interface),
            "zod" => Ok(Self::Zod),
            _ => Err(ScytheError::new(
                ErrorCode::InternalError,
                format!(
                    "invalid row_type '{}': expected 'interface' or 'zod'",
                    value
                ),
            )),
        }
    }
}

/// Map a neutral type to its Zod v4 schema expression.
/// Note: This does not handle enums - use column_to_zod for full column handling.
pub fn neutral_to_zod(neutral_type: &str, nullable: bool) -> String {
    let base = match neutral_type {
        "int16" | "int32" | "int64" => "z.number()",
        "float32" | "float64" => "z.number()",
        "string" | "text" | "inet" | "interval" | "time" | "time_tz" => "z.string()",
        "bool" => "z.boolean()",
        "datetime" | "datetime_tz" => "z.date()",
        "date" => "z.string()",
        "uuid" => "z.string().uuid()",
        "json" => "z.unknown()",
        "decimal" => "z.string()",
        "bytes" => "z.instanceof(Buffer)",
        t if t.starts_with("enum::") => "z.string()",
        _ => "z.unknown()",
    };
    if nullable {
        format!("{base}.nullable()")
    } else {
        base.to_string()
    }
}

/// Generate a Zod schema and inferred type for a row struct.
pub fn generate_zod_row_struct(
    struct_name: &str,
    query_name: &str,
    columns: &[ResolvedColumn],
) -> String {
    let schema_name = format!("{struct_name}Schema");
    let mut out = String::new();
    let _ = writeln!(out, "/** Row type for {} queries. */", query_name);
    let _ = writeln!(out, "export const {} = z.object({{", schema_name);
    for col in columns {
        let zod_type = column_to_zod(col);
        let _ = writeln!(out, "\t{}: {},", col.field_name, zod_type);
    }
    let _ = writeln!(out, "}});");
    let _ = writeln!(out);
    let _ = write!(
        out,
        "export type {} = z.infer<typeof {}>;",
        struct_name, schema_name
    );
    out
}

/// Map a ResolvedColumn to its Zod schema expression, handling enums properly.
fn column_to_zod(col: &ResolvedColumn) -> String {
    if col.neutral_type.starts_with("enum::") {
        // For enum types, extract the enum name from lang_type or compute it from neutral_type
        // lang_type should be like "UserStatus", we need "UserStatusSchema"
        let base = if col.lang_type.starts_with("enum::") {
            // If lang_type still has enum::, extract the enum name
            col.lang_type
                .strip_prefix("enum::")
                .unwrap_or(&col.lang_type)
                .to_string()
        } else {
            // lang_type should be the proper TypeScript enum name
            col.lang_type.clone()
        };
        let schema_name = format!("{}Schema", base);
        if col.nullable {
            format!("{schema_name}.nullable()")
        } else {
            schema_name
        }
    } else {
        neutral_to_zod(&col.neutral_type, col.nullable)
    }
}

/// Generate a Zod enum schema from enum values.
pub fn generate_zod_enum(type_name: &str, values: &[String]) -> String {
    let schema_name = format!("{type_name}Schema");
    let mut out = String::new();
    let variants: Vec<String> = values.iter().map(|v| format!("\"{}\"", v)).collect();
    let _ = writeln!(
        out,
        "export const {} = z.enum([{}]);",
        schema_name,
        variants.join(", ")
    );
    let _ = writeln!(out);
    let _ = write!(
        out,
        "export type {} = z.infer<typeof {}>;",
        type_name, schema_name
    );
    let _ = writeln!(out);
    let _ = writeln!(out);
    let _ = writeln!(out, "export const {} = {{", type_name);
    for value in values {
        let key = scythe_backend::naming::to_pascal_case(value);
        let _ = writeln!(out, "\t{}: \"{}\",", key, value);
    }
    let _ = write!(out, "}} as const;");
    out
}
