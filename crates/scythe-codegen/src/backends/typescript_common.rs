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

/// Render the plain TypeScript interface for a row.
///
/// Shared by every TypeScript backend so the shape cannot drift between them.
pub fn generate_ts_interface_row_struct(struct_name: &str, query_name: &str, columns: &[ResolvedColumn]) -> String {
    generate_ts_interface_row_struct_with_base(struct_name, query_name, columns, None)
}

/// As [`generate_ts_interface_row_struct`], but extending `base` when the
/// target requires it (mysql2's `RowDataPacket`).
pub fn generate_ts_interface_row_struct_with_base(
    struct_name: &str,
    query_name: &str,
    columns: &[ResolvedColumn],
    base: Option<&str>,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "/** Row type for {} queries. */", query_name);
    match base {
        Some(base) => {
            let _ = writeln!(out, "export interface {} extends {} {{", struct_name, base);
        }
        None => {
            let _ = writeln!(out, "export interface {} {{", struct_name);
        }
    }
    for col in columns {
        let _ = writeln!(out, "\t{}: {};", col.field_name, col.full_type);
    }
    let _ = write!(out, "}}");
    out
}

/// Group the columns that an outer join can null out together.
///
/// Returns the groups in first-appearance order, keeping only those with at
/// least one discriminant — a column that was `NOT NULL` in the schema and so
/// can only be null when the join found no row. A group without a discriminant
/// carries no information a union could express, because every column in it was
/// independently nullable anyway.
fn discriminated_join_groups(columns: &[ResolvedColumn]) -> Vec<String> {
    let mut groups: Vec<String> = Vec::new();
    for col in columns {
        if let Some(group) = &col.join_group
            && !groups.contains(group)
            && columns
                .iter()
                .any(|c| c.join_group.as_ref() == Some(group) && c.is_join_discriminant())
        {
            groups.push(group.clone());
        }
    }
    groups
}

/// Render a row type that expresses outer-join nullability as a discriminated
/// union instead of independent per-column optionals.
///
/// For a `LEFT JOIN` where the joined relation projects at least one `NOT NULL`
/// column, the flat interface is sound but imprecise: it admits rows the query
/// can never produce. Given
///
/// ```sql
/// SELECT u.id, u.name, o.total, o.notes
/// FROM users u LEFT JOIN orders o ON u.id = o.user_id
/// ```
///
/// where `orders.total` is `NOT NULL` and `orders.notes` is not, a flat shape
/// admits `{ total: null, notes: "gift" }` — unreachable, because `total` is
/// null exactly when no order matched, and then `notes` is null too.
///
/// Columns from the inner side stay in a base object; each discriminated join
/// group contributes one matched/unmatched alternative.
///
/// Falls back to the plain interface when there is nothing to discriminate, so
/// callers can use this unconditionally.
/// `base` is intersected into the result when the target requires the row to
/// extend a driver type (mysql2's `RowDataPacket`). A union cannot use
/// `extends`, so it is expressed as an intersection instead.
pub fn generate_ts_union_row_struct(
    struct_name: &str,
    query_name: &str,
    columns: &[ResolvedColumn],
    base: Option<&str>,
) -> String {
    let groups = discriminated_join_groups(columns);
    if groups.is_empty() {
        return generate_ts_interface_row_struct_with_base(struct_name, query_name, columns, base);
    }

    let mut out = String::new();
    let _ = writeln!(out, "/** Row type for {} queries. */", query_name);
    match base {
        Some(base) => {
            let _ = writeln!(out, "export type {} = {} & {{", struct_name, base);
        }
        None => {
            let _ = writeln!(out, "export type {} = {{", struct_name);
        }
    }

    for col in columns.iter().filter(|c| c.join_group.is_none()) {
        let _ = writeln!(out, "\t{}: {};", col.field_name, col.full_type);
    }
    let _ = write!(out, "}}");

    for group in &groups {
        let members: Vec<&ResolvedColumn> = columns
            .iter()
            .filter(|c| c.join_group.as_ref() == Some(group))
            .collect();

        let _ = writeln!(out, " & (");

        // Matched: the join found a row, so each column takes its own
        // schema nullability.
        let _ = write!(out, "\t| {{ ");
        for (index, col) in members.iter().enumerate() {
            if index > 0 {
                let _ = write!(out, "; ");
            }
            let matched_type = if col.nullable_before_join {
                col.full_type.as_str()
            } else {
                col.lang_type.as_str()
            };
            let _ = write!(out, "{}: {}", col.field_name, matched_type);
        }
        let _ = writeln!(out, " }}");

        // Unmatched: no row on the outer side, so every projected column is
        // null together.
        let _ = write!(out, "\t| {{ ");
        for (index, col) in members.iter().enumerate() {
            if index > 0 {
                let _ = write!(out, "; ");
            }
            let _ = write!(out, "{}: null", col.field_name);
        }
        let _ = writeln!(out, " }}");

        let _ = write!(out, ")");
    }

    // A type alias needs the terminator; the interface form does not.
    let _ = write!(out, ";");
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    fn column(
        name: &str,
        lang_type: &str,
        nullable: bool,
        group: Option<&str>,
        nullable_before_join: bool,
    ) -> ResolvedColumn {
        ResolvedColumn {
            name: name.to_string(),
            field_name: name.to_string(),
            lang_type: lang_type.to_string(),
            full_type: if nullable {
                format!("{lang_type} | null")
            } else {
                lang_type.to_string()
            },
            neutral_type: "string".to_string(),
            nullable,
            join_group: group.map(str::to_string),
            nullable_before_join,
        }
    }

    /// The example from the issue: `orders.total` is NOT NULL so it
    /// discriminates the join, while `orders.notes` is independently nullable.
    fn user_orders_columns() -> Vec<ResolvedColumn> {
        vec![
            column("id", "number", false, None, false),
            column("name", "string", false, None, false),
            column("total", "string", true, Some("o"), false),
            column("notes", "string", true, Some("o"), true),
        ]
    }

    #[test]
    fn emits_a_union_that_ties_the_outer_join_columns_together() {
        let out = generate_ts_union_row_struct("GetUserOrdersRow", "GetUserOrders", &user_orders_columns(), None);

        assert!(out.contains("export type GetUserOrdersRow = {"), "{out}");
        // Inner-side columns stay in the base object.
        assert!(out.contains("\tid: number;"), "{out}");
        assert!(out.contains("\tname: string;"), "{out}");
        // Matched: total is non-null, notes keeps its own nullability.
        assert!(out.contains("| { total: string; notes: string | null }"), "{out}");
        // Unmatched: the whole group is null together.
        assert!(out.contains("| { total: null; notes: null }"), "{out}");
        assert!(out.ends_with(");"), "type alias must be terminated: {out}");
    }

    /// Without a NOT NULL column on the outer side there is no discriminant,
    /// so the flat shape is already exact and a union would add nothing.
    #[test]
    fn falls_back_to_the_flat_interface_when_no_column_discriminates() {
        let columns = vec![
            column("id", "number", false, None, false),
            column("notes", "string", true, Some("o"), true),
        ];

        let out = generate_ts_union_row_struct("R", "Q", &columns, None);

        assert!(out.contains("export interface R {"), "{out}");
        assert!(!out.contains(" & ("), "no union without a discriminant: {out}");
    }

    #[test]
    fn falls_back_to_the_flat_interface_when_there_is_no_outer_join() {
        let columns = vec![
            column("id", "number", false, None, false),
            column("name", "string", false, None, false),
        ];

        let out = generate_ts_union_row_struct("R", "Q", &columns, None);

        assert_eq!(out, generate_ts_interface_row_struct("R", "Q", &columns));
    }

    /// mysql2 rows must extend RowDataPacket. A union cannot use `extends`, so
    /// the base is intersected instead.
    #[test]
    fn intersects_the_base_type_when_the_target_requires_one() {
        let out = generate_ts_union_row_struct("R", "Q", &user_orders_columns(), Some("RowDataPacket"));

        assert!(out.contains("export type R = RowDataPacket & {"), "{out}");
    }

    /// Two independently outer-joined relations each get their own
    /// match/no-match alternative rather than being conflated.
    #[test]
    fn emits_one_alternative_per_join_group() {
        let columns = vec![
            column("id", "number", false, None, false),
            column("total", "string", true, Some("o"), false),
            column("addr", "string", true, Some("a"), false),
        ];

        let out = generate_ts_union_row_struct("R", "Q", &columns, None);

        assert!(out.contains("| { total: string }"), "{out}");
        assert!(out.contains("| { total: null }"), "{out}");
        assert!(out.contains("| { addr: string }"), "{out}");
        assert!(out.contains("| { addr: null }"), "{out}");
        assert_eq!(out.matches(" & (").count(), 2, "one group per relation: {out}");
    }
}
