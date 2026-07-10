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
                format!("invalid row_type '{}': expected 'interface' or 'zod'", value),
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
pub fn generate_zod_row_struct(struct_name: &str, query_name: &str, columns: &[ResolvedColumn]) -> String {
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
    let _ = write!(out, "export type {} = z.infer<typeof {}>;", struct_name, schema_name);
    out
}

/// Map a ResolvedColumn to its Zod schema expression, handling enums properly.
fn column_to_zod(col: &ResolvedColumn) -> String {
    if col.neutral_type.starts_with("enum::") {
        let base = if col.lang_type.starts_with("enum::") {
            col.lang_type
                .strip_prefix("enum::")
                .unwrap_or(&col.lang_type)
                .to_string()
        } else {
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

/// Generate paired child + parent TypeScript interfaces for a `:grouped` query.
///
/// Child interface is emitted first so the parent's `children: ChildType[]` field
/// resolves without a forward reference.
pub fn generate_grouped_interface_structs(
    child_struct_name: &str,
    parent_struct_name: &str,
    parent_columns: &[ResolvedColumn],
    child_columns: &[ResolvedColumn],
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "/** Child row type for grouped query. */");
    let _ = writeln!(out, "export interface {child_struct_name} {{");
    for col in child_columns {
        let _ = writeln!(out, "\t{}: {};", col.field_name, col.full_type);
    }
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "/** Parent row type for grouped query. */");
    let _ = writeln!(out, "export interface {parent_struct_name} {{");
    for col in parent_columns {
        let _ = writeln!(out, "\t{}: {};", col.field_name, col.full_type);
    }
    let _ = writeln!(out, "\tchildren: {child_struct_name}[];");
    let _ = write!(out, "}}");
    out
}

/// Generate paired child + parent Zod schemas for a `:grouped` query.
///
/// Child schema is emitted first so the parent's `children: z.array(ChildSchema)`
/// reference resolves without a forward declaration.
pub fn generate_zod_grouped_structs(
    child_struct_name: &str,
    parent_struct_name: &str,
    parent_columns: &[ResolvedColumn],
    child_columns: &[ResolvedColumn],
) -> String {
    let child_schema = format!("{child_struct_name}Schema");
    let parent_schema = format!("{parent_struct_name}Schema");
    let mut out = String::new();
    let _ = writeln!(out, "/** Child row type for grouped query. */");
    let _ = writeln!(out, "export const {child_schema} = z.object({{");
    for col in child_columns {
        let zod = neutral_to_zod(&col.neutral_type, col.nullable);
        let _ = writeln!(out, "\t{}: {},", col.field_name, zod);
    }
    let _ = writeln!(out, "}});");
    let _ = writeln!(out);
    let _ = writeln!(out, "export type {child_struct_name} = z.infer<typeof {child_schema}>;");
    let _ = writeln!(out);
    let _ = writeln!(out, "/** Parent row type for grouped query. */");
    let _ = writeln!(out, "export const {parent_schema} = z.object({{");
    for col in parent_columns {
        let zod = neutral_to_zod(&col.neutral_type, col.nullable);
        let _ = writeln!(out, "\t{}: {},", col.field_name, zod);
    }
    let _ = writeln!(out, "\tchildren: z.array({child_schema}),");
    let _ = writeln!(out, "}});");
    let _ = writeln!(out);
    let _ = write!(
        out,
        "export type {parent_struct_name} = z.infer<typeof {parent_schema}>;"
    );
    out
}

/// Generate the client-side fold body for a `:grouped` query.
///
/// `row_access(sql_col_name, ts_full_type)` returns the TypeScript expression that
/// reads that column from the current `row` loop variable.  Examples:
/// - pg/mysql2 (rows are `Record<string, any>`): `|name, _| format!("row.{name}")`
/// - postgres.js/mssql/duckdb: `|name, ty| format!("row['{}'] as {}", name, ty)`
/// - Oracle (uppercase keys):  `|name, ty| format!("row['{}'] as {}", name.to_uppercase(), ty)`
///
/// The helper emits the fold loop into a string that is appended directly inside
/// the function body; the caller is responsible for surrounding braces.
pub fn generate_ts_grouped_fold_body(
    parent_struct_name: &str,
    _child_struct_name: &str,
    parent_columns: &[ResolvedColumn],
    child_columns: &[ResolvedColumn],
    key_col_name: &str,
    row_access: impl Fn(&str, &str) -> String,
) -> String {
    let key_type = parent_columns
        .iter()
        .find(|c| c.name == key_col_name)
        .map_or("unknown", |c| c.full_type.as_str());

    let mut out = String::new();
    let _ = writeln!(out, "\tconst result: {parent_struct_name}[] = [];");
    let _ = writeln!(out, "\tconst index = new Map<unknown, {parent_struct_name}>();");
    let _ = writeln!(out, "\tfor (const row of flatRows) {{");
    let _ = writeln!(out, "\t\tconst key = {};", row_access(key_col_name, key_type));
    let _ = writeln!(out, "\t\tlet parent = index.get(key);");
    let _ = writeln!(out, "\t\tif (!parent) {{");
    let _ = writeln!(out, "\t\t\tparent = {{");
    for col in parent_columns {
        let _ = writeln!(
            out,
            "\t\t\t\t{}: {},",
            col.field_name,
            row_access(&col.name, &col.full_type)
        );
    }
    let _ = writeln!(out, "\t\t\t\tchildren: [],");
    let _ = writeln!(out, "\t\t\t}};");
    let _ = writeln!(out, "\t\t\tindex.set(key, parent);");
    let _ = writeln!(out, "\t\t\tresult.push(parent);");
    let _ = writeln!(out, "\t\t}}");
    let _ = writeln!(out, "\t\tparent.children.push({{");
    for col in child_columns {
        let _ = writeln!(
            out,
            "\t\t\t{}: {},",
            col.field_name,
            row_access(&col.name, &col.full_type)
        );
    }
    let _ = writeln!(out, "\t\t}});");
    let _ = writeln!(out, "\t}}");
    let _ = writeln!(out, "\treturn result;");
    out
}

/// Generate a Zod enum schema from enum values.
pub fn generate_zod_enum(type_name: &str, values: &[String]) -> String {
    let schema_name = format!("{type_name}Schema");
    let mut out = String::new();
    let variants: Vec<String> = values.iter().map(|v| format!("\"{}\"", v)).collect();
    let _ = writeln!(out, "export const {} = z.enum([{}]);", schema_name, variants.join(", "));
    let _ = writeln!(out);
    let _ = write!(out, "export type {} = z.infer<typeof {}>;", type_name, schema_name);
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
