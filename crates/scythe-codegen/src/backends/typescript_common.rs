use std::collections::HashMap;
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

/// Case convention for row/interface field names, as selected by the
/// `field_case` backend option.
///
/// Naming the field (`NamingConfig.field_case`, read centrally in
/// `resolve.rs`) and remapping the *runtime* row to match it are two
/// separate concerns: renaming alone would type-check but return `undefined`
/// for every field once `Camel` disagrees with what the driver actually
/// returns. This enum drives the latter -- see
/// [`generate_ts_one_row_remap`] and [`generate_ts_many_row_remap`], which a
/// backend's `apply_options` selects between based on the same
/// `field_case` option string that also sets `NamingConfig.field_case`, so
/// the two never drift apart.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TsFieldCase {
    #[default]
    Snake,
    Camel,
}

impl TsFieldCase {
    /// Parse a field_case option string into a `TsFieldCase`.
    pub fn from_option(value: &str) -> Result<Self, ScytheError> {
        match value {
            "snake_case" => Ok(Self::Snake),
            "camelCase" => Ok(Self::Camel),
            _ => Err(ScytheError::new(
                ErrorCode::InternalError,
                format!("invalid field_case '{}': expected 'snake_case' or 'camelCase'", value),
            )),
        }
    }
}

/// Which declared row-type shape a reconstructed row object literal has to
/// satisfy.
///
/// A remap builds a plain object literal and returns it against the query
/// function's declared row type, so the cast on each field has to agree with
/// what that type declares for it. The two shapes disagree about exactly one
/// class of column — see [`TsRowShape::cast_type`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TsRowShape {
    /// Plain interface (`outer_join_unions = false`, and every `:grouped`
    /// parent/child struct): every field is declared at `full_type`.
    #[default]
    Flat,
    /// Discriminated union (`outer_join_unions = true`): see
    /// [`generate_ts_union_row_struct`].
    Union,
}

impl TsRowShape {
    /// The shape a backend declares given its `outer_join_unions` setting.
    pub fn from_outer_join_unions(outer_join_unions: bool) -> Self {
        if outer_join_unions { Self::Union } else { Self::Flat }
    }

    /// The TypeScript type a reconstructed field must be cast to so the
    /// object literal stays assignable to the declared row type.
    ///
    /// `full_type` everywhere except a join discriminant under
    /// [`TsRowShape::Union`]. There, [`generate_ts_union_row_struct`]
    /// declares the column as `lang_type` in the matched variant and as
    /// literal `null` in the unmatched one, and `T | null` is assignable to
    /// neither — casting through `full_type` is a TS2322, not a widening.
    /// `lang_type` would stay sound under `Flat` as well (`T` is assignable
    /// to `T | null`), but `Flat` keeps `full_type` so the generated text is
    /// unchanged for the shape that never needed the narrowing.
    pub fn cast_type(self, col: &ResolvedColumn) -> &str {
        if self == Self::Union && col.is_join_discriminant() {
            return &col.lang_type;
        }
        &col.full_type
    }
}

/// Parse a boolean backend option strictly.
///
/// Accepts (case-insensitively) `true`/`false`, `1`/`0`, and `yes`/`no`. Any
/// other value — a typo, `"on"`, an empty string — is rejected with an error
/// naming the option and the offending value, rather than being silently
/// coerced to `false` as `matches!` or `== "true"` would do.
pub fn parse_bool_option(option_name: &str, value: &str) -> Result<bool, ScytheError> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(ScytheError::new(
            ErrorCode::InternalError,
            format!("invalid {option_name} '{value}': expected 'true'/'false', '1'/'0', or 'yes'/'no'"),
        )),
    }
}

/// Reject any key in `options` that is not in `known`.
///
/// Before this, every TypeScript `apply_options` override just did
/// `options.get("known_key")` and returned `Ok(())` by default (the
/// `CodegenBackend::apply_options` default impl) -- an unrecognised key like
/// a typo'd `row_typ = "zod"`, or a real key that TOML happily parses but no
/// override reads, was silently discarded. The manifest author would see no
/// error and no effect. Callers run this as the first line of
/// `apply_options`, before touching any individual option, so a typo is
/// reported instead of ignored.
///
/// When a rejected key is within edit distance 2 of a known one, the error
/// suggests it -- close enough to catch `row_typ` -> `row_type` or
/// `outer_join_union` -> `outer_join_unions` without false-positiving on
/// genuinely unrelated keys.
pub fn reject_unknown_options(known: &[&str], options: &HashMap<String, String>) -> Result<(), ScytheError> {
    let mut keys: Vec<&String> = options.keys().collect();
    keys.sort();

    for key in keys {
        if known.contains(&key.as_str()) {
            continue;
        }

        let suggestion = known
            .iter()
            .map(|&candidate| (candidate, levenshtein_distance(key, candidate)))
            .filter(|&(_, distance)| distance <= 2)
            .min_by_key(|&(_, distance)| distance)
            .map(|(candidate, _)| candidate);

        let message = match suggestion {
            Some(suggestion) => format!(
                "unknown option '{key}' (did you mean '{suggestion}'?): valid options are {}",
                known.join(", ")
            ),
            None => format!("unknown option '{key}': valid options are {}", known.join(", ")),
        };
        return Err(ScytheError::new(ErrorCode::InternalError, message));
    }

    Ok(())
}

/// Levenshtein edit distance between two strings, used by
/// [`reject_unknown_options`] to suggest a likely intended option name.
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();

    let mut prev_row: Vec<usize> = (0..=b.len()).collect();
    let mut curr_row = vec![0usize; b.len() + 1];

    for (i, &char_a) in a.iter().enumerate() {
        curr_row[0] = i + 1;
        for (j, &char_b) in b.iter().enumerate() {
            let substitution_cost = usize::from(char_a != char_b);
            curr_row[j + 1] = (prev_row[j + 1] + 1)
                .min(curr_row[j] + 1)
                .min(prev_row[j] + substitution_cost);
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b.len()]
}

/// Escape a SQL string for safe splicing into a JS backtick template
/// literal.
///
/// Order matters: backslash must be escaped first, so the backslashes this
/// pass inserts ahead of a backtick or `${` are not themselves re-escaped by
/// a later pass. Left unescaped, a backslash in the source SQL (e.g.
/// `E'\n'`) would escape whatever JS character follows it; a literal
/// backtick (idiomatic identifier quoting in MySQL/MariaDB — `` `users`.`id` ``)
/// would terminate the template literal early; and a literal `${` would open
/// a live JS interpolation — worse still inside a Kysely `sql` tag, where
/// `${}` is a parameter binding.
pub fn escape_ts_template_literal(sql: &str) -> String {
    sql.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${")
}

/// Escape a SQL string for safe splicing into a double-quoted JS string
/// literal.
///
/// Used by the oracledb backend, which binds SQL text as a plain
/// double-quoted string rather than a template literal, so `` ` `` and `${`
/// are inert there but a literal `"` would terminate the string early.
/// Backslash is escaped first for the same reason as
/// [`escape_ts_template_literal`].
pub fn escape_ts_double_quoted_literal(sql: &str) -> String {
    sql.replace('\\', "\\\\").replace('"', "\\\"")
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

    // Not just `join_group.is_none()`: a column belonging to a join group
    // that `discriminated_join_groups` dropped -- one where every projected
    // column was already nullable in the schema -- has no union variant to
    // live in, so filtering on `is_none()` alone omitted it from the row
    // type entirely. A query with two LEFT JOINs where only one joined
    // relation projects a NOT NULL column selected five columns and got a
    // type declaring three: silent on `typescript-pg`, which hands driver
    // rows back directly, and a compile error on the nine remap backends,
    // whose object literal then assigns properties the type does not
    // declare. Such a column is independently nullable, so `full_type` is
    // exactly right for it -- which is also what `TsRowShape::cast_type`
    // gives it, since it is not a join discriminant.
    for col in columns
        .iter()
        .filter(|c| c.join_group.as_ref().is_none_or(|group| !groups.contains(group)))
    {
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
    column_to_zod_with_nullable(col, col.nullable)
}

/// As [`column_to_zod`], but with an explicit nullability instead of
/// `col.nullable`.
///
/// Needed by [`generate_zod_union_row_struct`]: inside the "matched" branch
/// of a discriminated union, a join-group column's schema nullability is
/// `col.nullable_before_join` (its own nullability, independent of the
/// join), not `col.nullable` (which the analyzer has already widened to
/// `true` for every column in the group).
fn column_to_zod_with_nullable(col: &ResolvedColumn, nullable: bool) -> String {
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
        if nullable {
            format!("{schema_name}.nullable()")
        } else {
            schema_name
        }
    } else if col.neutral_type == "bytes" {
        // The runtime type of a binary column is backend-specific: `Buffer` for
        // the Node driver backends, `Uint8Array` for node:sqlite and
        // sqlite-wasm. Hardcoding `Buffer` mis-validates on those two and is
        // unresolvable in a browser, where `Buffer` does not exist at all.
        // `lang_type` already carries the manifest's mapping, so use it.
        let base = format!("z.instanceof({})", col.lang_type);
        if nullable { format!("{base}.nullable()") } else { base }
    } else {
        neutral_to_zod(&col.neutral_type, nullable)
    }
}

/// Render a Zod schema that expresses outer-join nullability as a
/// discriminated union instead of independent per-column optionals.
///
/// This is the Zod counterpart to [`generate_ts_union_row_struct`]: the
/// inferred type of the generated schema matches the shape that function
/// produces. Grouping is computed by the same [`discriminated_join_groups`]
/// helper so the two can never drift apart on which joins qualify.
///
/// Falls back to [`generate_zod_row_struct`] when there is nothing to
/// discriminate, so callers can use this unconditionally — matching the
/// fallback behavior of `generate_ts_union_row_struct`.
pub fn generate_zod_union_row_struct(struct_name: &str, query_name: &str, columns: &[ResolvedColumn]) -> String {
    let groups = discriminated_join_groups(columns);
    if groups.is_empty() {
        return generate_zod_row_struct(struct_name, query_name, columns);
    }

    let schema_name = format!("{struct_name}Schema");
    let mut out = String::new();
    let _ = writeln!(out, "/** Row type for {} queries. */", query_name);
    let _ = writeln!(out, "export const {} = z.object({{", schema_name);
    // Not just `join_group.is_none()`, for the same reason as the interface
    // path above: a column in a group `discriminated_join_groups` dropped --
    // one where every projected column was already nullable in the schema --
    // gets no union variant, so filtering on `is_none()` alone left it out of
    // the schema entirely and `z.infer` produced a type missing that field.
    for col in columns
        .iter()
        .filter(|c| c.join_group.as_ref().is_none_or(|group| !groups.contains(group)))
    {
        let zod_type = column_to_zod(col);
        let _ = writeln!(out, "\t{}: {},", col.field_name, zod_type);
    }
    let _ = write!(out, "}})");

    for group in &groups {
        let members: Vec<&ResolvedColumn> = columns
            .iter()
            .filter(|c| c.join_group.as_ref() == Some(group))
            .collect();

        // Matched: the join found a row, so each column takes its own
        // pre-join schema nullability.
        let matched_fields: Vec<String> = members
            .iter()
            .map(|col| {
                format!(
                    "{}: {}",
                    col.field_name,
                    column_to_zod_with_nullable(col, col.nullable_before_join)
                )
            })
            .collect();
        // Unmatched: no row on the outer side, so every projected column is
        // null together.
        let unmatched_fields: Vec<String> = members
            .iter()
            .map(|col| format!("{}: z.null()", col.field_name))
            .collect();

        let _ = write!(
            out,
            ".and(z.union([z.object({{ {} }}), z.object({{ {} }})]))",
            matched_fields.join(", "),
            unmatched_fields.join(", "),
        );
    }
    let _ = writeln!(out, ";");
    let _ = writeln!(out);
    let _ = write!(out, "export type {} = z.infer<typeof {}>;", struct_name, schema_name);
    out
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
        let zod = column_to_zod(col);
        let _ = writeln!(out, "\t{}: {},", col.field_name, zod);
    }
    let _ = writeln!(out, "}});");
    let _ = writeln!(out);
    let _ = writeln!(out, "export type {child_struct_name} = z.infer<typeof {child_schema}>;");
    let _ = writeln!(out);
    let _ = writeln!(out, "/** Parent row type for grouped query. */");
    let _ = writeln!(out, "export const {parent_schema} = z.object({{");
    for col in parent_columns {
        let zod = column_to_zod(col);
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

/// Render the `field: value,` lines of a TypeScript object literal for a
/// row's columns, one per line, at the given `indent`.
///
/// `row_access(sql_col_name, ts_full_type)` returns the TypeScript expression
/// that reads that column from the current row variable -- see
/// [`generate_ts_grouped_fold_body`] for the per-backend examples.
///
/// `shape` picks the type each field is cast to, so the literal stays
/// assignable to the row type the caller declared -- see
/// [`TsRowShape::cast_type`].
///
/// Shared by both halves of `generate_ts_grouped_fold_body` -- the parent
/// object and the child object it pushes into `children` -- so their field
/// lists cannot drift apart. Also used by [`generate_ts_one_row_remap`] and
/// [`generate_ts_many_row_remap`] for `:one`/`:many` row construction under
/// `field_case = "camelCase"`. Callers own the surrounding `{ ... }` and any
/// additional properties (e.g. `children: []`), since those differ between
/// call sites.
pub fn generate_ts_row_object_literal(
    columns: &[ResolvedColumn],
    indent: &str,
    shape: TsRowShape,
    row_access: impl Fn(&str, &str) -> String,
) -> String {
    let mut out = String::new();
    for col in columns {
        let _ = writeln!(
            out,
            "{indent}{}: {},",
            col.field_name,
            row_access(&col.name, shape.cast_type(col))
        );
    }
    out
}

/// Render a `:one`/`:opt` return body that reconstructs the row field by
/// field instead of trusting a blind cast of the driver's raw row.
///
/// Assumes the caller has already bound the raw driver row (or
/// `undefined`/`null`, if none matched) to a local `row`. Null-checks it,
/// then builds the return value via [`generate_ts_row_object_literal`],
/// using the same `row_access` closure the backend already passes to
/// [`generate_ts_grouped_fold_body`] to read a named column off a raw row.
///
/// A blind `as StructName` cast of the driver's row (what every `:one`/
/// `:opt` used before this existed, and still does under the default
/// `field_case = "snake_case"`, where it is sound) is unsound once
/// `field_case = "camelCase"` renames the declared fields: the driver still
/// returns snake_case keys, so `tsc` reports no error while every field
/// reads back `undefined` at runtime.
///
/// `shape` must be the shape of the row type the enclosing function
/// declares, so the literal is assignable to it -- see
/// [`TsRowShape::cast_type`].
pub fn generate_ts_one_row_remap(
    columns: &[ResolvedColumn],
    shape: TsRowShape,
    row_access: impl Fn(&str, &str) -> String,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "\tif (!row) {{");
    let _ = writeln!(out, "\t\treturn null;");
    let _ = writeln!(out, "\t}}");
    let _ = writeln!(out, "\treturn {{");
    out.push_str(&generate_ts_row_object_literal(columns, "\t\t", shape, row_access));
    let _ = writeln!(out, "\t}};");
    out
}

/// Render a `:many` return body that reconstructs each row field by field
/// instead of trusting a blind cast of the driver's raw rows.
///
/// Assumes the caller has already bound the raw driver rows to a local
/// `rows` array. Maps each one (bound to `row` inside the callback) through
/// [`generate_ts_row_object_literal`]. See [`generate_ts_one_row_remap`] for
/// why this exists instead of a blind cast, and for what `shape` selects.
pub fn generate_ts_many_row_remap(
    columns: &[ResolvedColumn],
    shape: TsRowShape,
    row_access: impl Fn(&str, &str) -> String,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "\treturn rows.map((row) => ({{");
    out.push_str(&generate_ts_row_object_literal(columns, "\t\t", shape, row_access));
    let _ = writeln!(out, "\t}}));");
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
///
/// The parent and child objects are always built at [`TsRowShape::Flat`]:
/// `:grouped` declares its structs through
/// [`generate_grouped_interface_structs`] (or its Zod twin), which emits a
/// plain object per struct at `full_type` regardless of the backend's
/// `outer_join_unions` setting, so there is no union variant here to agree
/// with.
///
/// `js_mode` drops the `: Type` local-variable annotations and the
/// `Map<unknown, Type>` generic argument, neither of which plain JavaScript
/// can parse -- the JSDoc-mode counterpart used by the `javascript-*` emit
/// mode. The fold logic itself (the `Map`-based grouping loop) is identical
/// either way, so this stays one function rather than a duplicated one.
pub fn generate_ts_grouped_fold_body(
    parent_struct_name: &str,
    _child_struct_name: &str,
    parent_columns: &[ResolvedColumn],
    child_columns: &[ResolvedColumn],
    key_col_name: &str,
    js_mode: bool,
    row_access: impl Fn(&str, &str) -> String,
) -> String {
    let key_type = parent_columns
        .iter()
        .find(|c| c.name == key_col_name)
        .map_or("unknown", |c| c.full_type.as_str());

    let mut out = String::new();
    if js_mode {
        let _ = writeln!(out, "\tconst result = [];");
        let _ = writeln!(out, "\tconst index = new Map();");
    } else {
        let _ = writeln!(out, "\tconst result: {parent_struct_name}[] = [];");
        let _ = writeln!(out, "\tconst index = new Map<unknown, {parent_struct_name}>();");
    }
    let _ = writeln!(out, "\tfor (const row of flatRows) {{");
    let _ = writeln!(out, "\t\tconst key = {};", row_access(key_col_name, key_type));
    let _ = writeln!(out, "\t\tlet parent = index.get(key);");
    let _ = writeln!(out, "\t\tif (!parent) {{");
    let _ = writeln!(out, "\t\t\tparent = {{");
    out.push_str(&generate_ts_row_object_literal(
        parent_columns,
        "\t\t\t\t",
        TsRowShape::Flat,
        &row_access,
    ));
    let _ = writeln!(out, "\t\t\t\tchildren: [],");
    let _ = writeln!(out, "\t\t\t}};");
    let _ = writeln!(out, "\t\t\tindex.set(key, parent);");
    let _ = writeln!(out, "\t\t\tresult.push(parent);");
    let _ = writeln!(out, "\t\t}}");
    let _ = writeln!(out, "\t\tparent.children.push({{");
    out.push_str(&generate_ts_row_object_literal(
        child_columns,
        "\t\t\t",
        TsRowShape::Flat,
        &row_access,
    ));
    let _ = writeln!(out, "\t\t}});");
    let _ = writeln!(out, "\t}}");
    let _ = writeln!(out, "\treturn result;");
    out
}

/// Render one JSDoc `@property` line for a row typedef.
///
/// This function's source is grepped by a dedicated test
/// (`test_js_property_line_source_cannot_emit_optional_syntax_into_the_name_position`)
/// to guarantee no future edit can reintroduce JSDoc's bracket-optional
/// (`[name]`) or `?`-suffix property syntax here. A nullable SQL column is a
/// property that is always present on the row and may hold `null` --
/// `{T | null}` -- never a property that may be *absent*, which is what
/// `[name]`/`name?` mean in JSDoc. `col.full_type` already carries the
/// `| null` suffix when `col.nullable` is true (computed once, upstream, by
/// the type resolver -- see [`ResolvedColumn`]), so this function has no
/// reason to branch on `col.nullable` at all: there is deliberately no such
/// conditional here for the grep guard to catch a regression of.
// jsdoc-property-line-start
pub fn js_property_line(col: &ResolvedColumn) -> String {
    format!(" * @property {{{}}} {}", col.full_type, col.field_name)
}
// jsdoc-property-line-end

/// Render a plain-JS JSDoc `@typedef` for a row -- the JSDoc-mode
/// counterpart of [`generate_ts_interface_row_struct`], used by the
/// `javascript-*` emit mode of the TypeScript backends (#81). Every column
/// goes through [`js_property_line`], so the nullable-as-`T | null` rule
/// applies uniformly here too.
pub fn generate_js_typedef_row_struct(struct_name: &str, query_name: &str, columns: &[ResolvedColumn]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "/**");
    let _ = writeln!(out, " * Row type for {} queries.", query_name);
    let _ = writeln!(out, " * @typedef {{object}} {}", struct_name);
    for col in columns {
        let _ = writeln!(out, "{}", js_property_line(col));
    }
    let _ = write!(out, " */");
    out
}

/// Render a generic JSDoc `@typedef` from arbitrary `(name, type)` fields --
/// used for `:batch` params objects, which are built from [`ResolvedParam`]s
/// rather than [`ResolvedColumn`]s and so carry no SQL nullability to encode.
pub fn generate_js_typedef(type_name: &str, description: &str, fields: &[(String, String)]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "/**");
    let _ = writeln!(out, " * {}", description);
    let _ = writeln!(out, " * @typedef {{object}} {}", type_name);
    for (name, ty) in fields {
        let _ = writeln!(out, " * @property {{{}}} {}", ty, name);
    }
    let _ = write!(out, " */");
    out
}

/// Render paired child + parent JSDoc `@typedef`s for a `:grouped` query --
/// the JSDoc-mode counterpart of [`generate_grouped_interface_structs`].
pub fn generate_js_grouped_typedef_structs(
    child_struct_name: &str,
    parent_struct_name: &str,
    parent_columns: &[ResolvedColumn],
    child_columns: &[ResolvedColumn],
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "/**");
    let _ = writeln!(out, " * Child row type for grouped query.");
    let _ = writeln!(out, " * @typedef {{object}} {}", child_struct_name);
    for col in child_columns {
        let _ = writeln!(out, "{}", js_property_line(col));
    }
    let _ = writeln!(out, " */");
    let _ = writeln!(out);
    let _ = writeln!(out, "/**");
    let _ = writeln!(out, " * Parent row type for grouped query.");
    let _ = writeln!(out, " * @typedef {{object}} {}", parent_struct_name);
    for col in parent_columns {
        let _ = writeln!(out, "{}", js_property_line(col));
    }
    let _ = writeln!(out, " * @property {{{}[]}} children", child_struct_name);
    let _ = write!(out, " */");
    out
}

/// Render a JSDoc inline type cast: `/** @type {T} */ (expr)`.
///
/// The JSDoc analog of a TypeScript `as T` assertion, for plain `.js` files
/// checked with `tsc --checkJs`. `as` is TS-only syntax and would make the
/// file fail to parse as JavaScript, so the `javascript-*` emit mode uses
/// this everywhere a TypeScript backend would write `expr as T`.
pub fn js_type_cast(ty: &str, expr: &str) -> String {
    format!("/** @type {{{ty}}} */ ({expr})")
}

/// Render a JSDoc block combining a one-line description, one `@param` tag
/// per `(name, type)` pair, and a `@returns` tag -- the JSDoc-mode
/// counterpart of a TypeScript function's inline `(name: Type): Ret`
/// annotations, which a plain `.js` file cannot carry on the signature
/// itself.
pub fn generate_jsdoc_fn_header(description: &str, sig_params: &[(String, String)], ret_type: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "/**");
    let _ = writeln!(out, " * {}", description);
    for (name, ty) in sig_params {
        let _ = writeln!(out, " * @param {{{}}} {}", ty, name);
    }
    let _ = writeln!(out, " * @returns {{{}}}", ret_type);
    let _ = write!(out, " */");
    out
}

/// Render a JS function signature line with plain parameter names and no
/// type annotations -- the JSDoc-mode counterpart of a TypeScript
/// `name: Type` signature (see [`generate_jsdoc_fn_header`] for where the
/// types go instead).
pub fn js_fn_signature_line(is_async: bool, name: &str, sig_params: &[(String, String)]) -> String {
    let params_inline = sig_params.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>().join(", ");
    let keyword = if is_async {
        "export async function"
    } else {
        "export function"
    };
    format!("{keyword} {name}({params_inline}) {{")
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
            sql_type: "text".to_string(),
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

    /// This must fail before the fix: a join group `discriminated_join_groups`
    /// drops -- every projected column already nullable in the schema -- got
    /// no union variant, and the base-field loop filtered on
    /// `join_group.is_none()`, so its columns were declared nowhere. The
    /// query below selects four columns; the row type declared two. The
    /// whole-query case is caught by the flat-interface fallback, so only a
    /// query mixing a discriminated group with an undiscriminated one
    /// reaches it.
    #[test]
    fn keeps_columns_from_join_groups_that_carry_no_discriminant() {
        let columns = vec![
            column("id", "number", false, None, false),
            // Undiscriminated group: nullable in the schema before any join.
            column("bio", "string", true, Some("p"), true),
            column("website", "string", true, Some("p"), true),
            // Discriminated group: NOT NULL in the schema, so it can only be
            // null when the join found no row.
            column("label", "string", true, Some("b"), false),
        ];

        let out = generate_ts_union_row_struct("R", "Q", &columns, None);

        assert!(out.contains("bio: string | null;"), "{out}");
        assert!(out.contains("website: string | null;"), "{out}");
        assert!(out.contains("| { label: string }"), "{out}");
        assert!(out.contains("| { label: null }"), "{out}");
        assert_eq!(
            out.matches(" & (").count(),
            1,
            "only the discriminated group becomes a union: {out}"
        );
    }

    /// The Zod counterpart of the case above. The doc comment on
    /// `generate_zod_union_row_struct` promises its inferred type matches the
    /// interface form, so the two base-field filters have to agree; they did
    /// not, and `z.infer` produced a type missing the undiscriminated group's
    /// columns while the interface declared them.
    #[test]
    fn zod_keeps_columns_from_join_groups_that_carry_no_discriminant() {
        let columns = vec![
            column("id", "number", false, None, false),
            column("bio", "string", true, Some("p"), true),
            column("website", "string", true, Some("p"), true),
            column("label", "string", true, Some("b"), false),
        ];

        let out = generate_zod_union_row_struct("R", "Q", &columns);

        assert!(out.contains("bio:"), "{out}");
        assert!(out.contains("website:"), "{out}");
        assert!(out.contains("label:"), "{out}");
    }

    #[test]
    fn escape_ts_template_literal_escapes_backtick_dollar_brace_and_backslash() {
        let out = escape_ts_template_literal(r"a`b${c}d\e");
        assert_eq!(out, r"a\`b\${c}d\\e");
    }

    /// Backslash must be escaped first: if `${` or `` ` `` were escaped
    /// before backslash, the backslash this pass inserts ahead of them
    /// would itself get doubled by the backslash pass, corrupting the
    /// escape sequence.
    #[test]
    fn escape_ts_template_literal_does_not_double_escape_inserted_backslashes() {
        let out = escape_ts_template_literal("`${x}");
        // Exactly one backslash ahead of each special character, not two.
        assert_eq!(out, r"\`\${x}");
    }

    #[test]
    fn escape_ts_template_literal_leaves_plain_sql_untouched() {
        let out = escape_ts_template_literal("SELECT id, name FROM users WHERE id = $1");
        assert_eq!(out, "SELECT id, name FROM users WHERE id = $1");
    }

    #[test]
    fn escape_ts_double_quoted_literal_escapes_quote_and_backslash() {
        let out = escape_ts_double_quoted_literal(r#"a"b\c"#);
        assert_eq!(out, r#"a\"b\\c"#);
    }

    #[test]
    fn parse_bool_option_accepts_true_spellings_case_insensitively() {
        for value in ["true", "True", "TRUE", "1", "yes", "Yes", "YES"] {
            assert!(
                parse_bool_option("outer_join_unions", value).unwrap(),
                "expected {value} to parse as true"
            );
        }
    }

    #[test]
    fn parse_bool_option_accepts_false_spellings_case_insensitively() {
        for value in ["false", "False", "FALSE", "0", "no", "No", "NO"] {
            assert!(
                !parse_bool_option("outer_join_unions", value).unwrap(),
                "expected {value} to parse as false"
            );
        }
    }

    /// This must fail before the fix: the old `matches!(value.as_str(), "true" | "1" | "yes")`
    /// silently mapped every one of these to `false` instead of reporting an error.
    #[test]
    fn parse_bool_option_rejects_unrecognized_values_with_an_error_naming_the_option() {
        for value in ["on", "maybe", ""] {
            let err =
                parse_bool_option("outer_join_unions", value).expect_err(&format!("expected '{value}' to be rejected"));
            let message = err.to_string();
            assert!(
                message.contains("outer_join_unions"),
                "error should name the option: {message}"
            );
            assert!(message.contains(value), "error should include the bad value: {message}");
        }
    }

    fn bytes_column(name: &str, lang_type: &str, nullable: bool) -> ResolvedColumn {
        ResolvedColumn {
            name: name.to_string(),
            field_name: name.to_string(),
            lang_type: lang_type.to_string(),
            full_type: if nullable {
                format!("{lang_type} | null")
            } else {
                lang_type.to_string()
            },
            neutral_type: "bytes".to_string(),
            sql_type: "bytea".to_string(),
            nullable,
            join_group: None,
            nullable_before_join: false,
        }
    }

    /// A binary column's runtime type is backend-specific. Hardcoding `Buffer`
    /// mis-validates on the backends whose driver yields `Uint8Array`, and is
    /// unresolvable in a browser where `Buffer` does not exist.
    #[test]
    fn test_zod_bytes_schema_follows_the_backend_runtime_type() {
        let buffer_schema =
            generate_zod_row_struct("GetBlobRow", "GetBlob", &[bytes_column("payload", "Buffer", false)]);
        assert!(
            buffer_schema.contains("z.instanceof(Buffer)"),
            "Buffer backends keep Buffer; got:\n{buffer_schema}"
        );

        let uint8_schema =
            generate_zod_row_struct("GetBlobRow", "GetBlob", &[bytes_column("payload", "Uint8Array", false)]);
        assert!(
            uint8_schema.contains("z.instanceof(Uint8Array)"),
            "Uint8Array backends must not emit Buffer; got:\n{uint8_schema}"
        );
        assert!(
            !uint8_schema.contains("Buffer"),
            "Uint8Array backends must not reference Buffer at all; got:\n{uint8_schema}"
        );
    }

    #[test]
    fn test_zod_bytes_schema_preserves_nullability() {
        let schema = generate_zod_row_struct("GetBlobRow", "GetBlob", &[bytes_column("payload", "Uint8Array", true)]);
        assert!(
            schema.contains("z.instanceof(Uint8Array).nullable()"),
            "nullable bytes must stay nullable; got:\n{schema}"
        );
    }

    /// The grouped Zod emitter used to call `neutral_to_zod` directly, which
    /// has no column context — so an enum column degraded to a bare
    /// `z.string()` and a bytes column always claimed `Buffer`.
    #[test]
    fn test_grouped_zod_structs_use_column_aware_schemas() {
        let mut enum_col = column("status", "UserStatus", false, None, false);
        enum_col.neutral_type = "enum::user_status".to_string();

        let grouped = generate_zod_grouped_structs(
            "GetUsersWithOrdersChildRow",
            "GetUsersWithOrdersRow",
            &[bytes_column("avatar", "Uint8Array", false)],
            &[enum_col],
        );

        assert!(
            grouped.contains("UserStatusSchema"),
            "grouped enum must use its enum schema, not z.string(); got:\n{grouped}"
        );
        assert!(
            grouped.contains("z.instanceof(Uint8Array)"),
            "grouped bytes must follow the backend type; got:\n{grouped}"
        );
    }

    #[test]
    fn test_ts_field_case_from_option_accepts_known_values() {
        assert_eq!(TsFieldCase::from_option("snake_case").unwrap(), TsFieldCase::Snake);
        assert_eq!(TsFieldCase::from_option("camelCase").unwrap(), TsFieldCase::Camel);
    }

    #[test]
    fn test_ts_field_case_from_option_rejects_unknown_values() {
        let err = TsFieldCase::from_option("PascalCase").expect_err("PascalCase is not a valid field_case");
        assert!(err.to_string().contains("field_case"), "{err}");
    }

    #[test]
    fn test_ts_field_case_default_is_snake() {
        assert_eq!(TsFieldCase::default(), TsFieldCase::Snake);
    }

    #[test]
    fn test_generate_ts_row_object_literal_matches_grouped_fold_field_lines() {
        let columns = vec![
            column("id", "number", false, None, false),
            column("name", "string", false, None, false),
        ];
        let out = generate_ts_row_object_literal(&columns, "\t\t\t\t", TsRowShape::Flat, |name, ty| {
            format!("row.{name} as {ty}")
        });
        assert_eq!(
            out,
            "\t\t\t\tid: row.id as number,\n\t\t\t\tname: row.name as string,\n"
        );
    }

    #[test]
    fn test_generate_ts_row_object_literal_empty_columns_is_empty() {
        let out = generate_ts_row_object_literal(&[], "\t", TsRowShape::Flat, |name, ty| format!("row.{name} as {ty}"));
        assert_eq!(out, "");
    }

    /// This must fail before the fix: the remap always cast through
    /// `full_type`, but `generate_ts_union_row_struct` declares a join
    /// discriminant as `lang_type` in the matched variant and as literal
    /// `null` in the unmatched one. `string | null` is assignable to neither
    /// `{ total: string }` nor `{ total: null }`, so
    /// `field_case = "camelCase"` combined with `outer_join_unions = true`
    /// emitted a row object that does not type-check (TS2322).
    #[test]
    fn test_generate_ts_row_object_literal_union_shape_casts_discriminant_to_matched_type() {
        let columns = user_orders_columns();
        let out = generate_ts_row_object_literal(&columns, "\t", TsRowShape::Union, |name, ty| {
            format!("row['{name}'] as {ty}")
        });
        assert_eq!(
            out,
            "\tid: row['id'] as number,\n\
             \tname: row['name'] as string,\n\
             \ttotal: row['total'] as string,\n\
             \tnotes: row['notes'] as string | null,\n"
        );
    }

    /// The union shape narrows the discriminant only; `Flat` must keep the
    /// nullable cast, since a plain interface declares it as `T | null`.
    #[test]
    fn test_generate_ts_row_object_literal_flat_shape_keeps_full_type() {
        let columns = user_orders_columns();
        let out = generate_ts_row_object_literal(&columns, "\t", TsRowShape::Flat, |name, ty| {
            format!("row['{name}'] as {ty}")
        });
        assert!(
            out.contains("total: row['total'] as string | null,"),
            "flat rows declare the discriminant nullable; got:\n{out}"
        );
    }

    #[test]
    fn test_ts_row_shape_from_outer_join_unions() {
        assert_eq!(TsRowShape::from_outer_join_unions(false), TsRowShape::Flat);
        assert_eq!(TsRowShape::from_outer_join_unions(true), TsRowShape::Union);
        assert_eq!(TsRowShape::default(), TsRowShape::Flat);
    }

    /// A union row type whose group has no discriminant collapses back to a
    /// plain interface (see `discriminated_join_groups`), and the cast has
    /// to collapse with it -- a column that was already nullable in the
    /// schema is declared `T | null` in the matched variant too.
    #[test]
    fn test_ts_row_shape_union_keeps_full_type_for_non_discriminants() {
        let columns = user_orders_columns();
        let notes = columns.iter().find(|c| c.name == "notes").unwrap();
        assert_eq!(TsRowShape::Union.cast_type(notes), "string | null");
        let name = columns.iter().find(|c| c.name == "name").unwrap();
        assert_eq!(TsRowShape::Union.cast_type(name), "string");
    }

    #[test]
    fn test_generate_ts_one_row_remap_null_checks_then_builds_fields() {
        let columns = vec![
            column("id", "number", false, None, false),
            column("name", "string", false, None, false),
        ];
        let out = generate_ts_one_row_remap(&columns, TsRowShape::Flat, |name, ty| format!("row['{name}'] as {ty}"));
        assert_eq!(
            out,
            "\tif (!row) {\n\
             \t\treturn null;\n\
             \t}\n\
             \treturn {\n\
             \t\tid: row['id'] as number,\n\
             \t\tname: row['name'] as string,\n\
             \t};\n"
        );
    }

    #[test]
    fn test_generate_ts_many_row_remap_maps_each_row() {
        let columns = vec![column("id", "number", false, None, false)];
        let out = generate_ts_many_row_remap(&columns, TsRowShape::Flat, |name, ty| format!("row['{name}'] as {ty}"));
        assert_eq!(
            out,
            "\treturn rows.map((row) => ({\n\
             \t\tid: row['id'] as number,\n\
             \t}));\n"
        );
    }

    fn known_options() -> &'static [&'static str] {
        &["row_type", "outer_join_unions", "structs_only", "field_case"]
    }

    #[test]
    fn test_reject_unknown_options_accepts_known_keys() {
        let mut options = HashMap::new();
        options.insert("row_type".to_string(), "zod".to_string());
        options.insert("field_case".to_string(), "camelCase".to_string());
        reject_unknown_options(known_options(), &options).unwrap();
    }

    #[test]
    fn test_reject_unknown_options_accepts_empty_map() {
        reject_unknown_options(known_options(), &HashMap::new()).unwrap();
    }

    /// This must fail before `reject_unknown_options` existed: the default
    /// `apply_options` returned `Ok(())` for any options map, so a typo like
    /// `row_typ = "zod"` silently parsed as valid TOML and had no effect.
    #[test]
    fn test_reject_unknown_options_rejects_unrecognized_key() {
        let mut options = HashMap::new();
        options.insert("row_typ".to_string(), "zod".to_string());
        let err = reject_unknown_options(known_options(), &options).expect_err("row_typ is not a known option");
        let message = err.to_string();
        assert!(message.contains("row_typ"), "{message}");
        assert!(
            message.contains("row_type"),
            "error should list valid options: {message}"
        );
    }

    #[test]
    fn test_reject_unknown_options_suggests_close_typo() {
        let mut options = HashMap::new();
        options.insert("row_typ".to_string(), "zod".to_string());
        let err = reject_unknown_options(known_options(), &options).expect_err("row_typ is not a known option");
        assert!(
            err.to_string().contains("did you mean 'row_type'?"),
            "expected a did-you-mean suggestion: {err}"
        );
    }

    #[test]
    fn test_reject_unknown_options_no_suggestion_when_too_far() {
        let mut options = HashMap::new();
        options.insert("completely_unrelated_option".to_string(), "x".to_string());
        let err = reject_unknown_options(known_options(), &options).expect_err("not a known option");
        assert!(
            !err.to_string().contains("did you mean"),
            "should not suggest anything this far off: {err}"
        );
    }

    /// Structural guard for #81's critical correctness rule: a nullable
    /// column must be emitted as `{T | null}`, never as JSDoc's
    /// bracket-optional (`[name]`) or `?`-suffix property syntax (which mean
    /// the property may be *absent*, not that it may be `null`).
    ///
    /// This does not check the emitted *text* (that is
    /// [`test_js_property_line_never_emits_bracket_or_question_mark_syntax`]
    /// below) -- it greps [`js_property_line`]'s own *source*, between the
    /// `jsdoc-property-line-start`/`-end` markers, for the literal
    /// characters `[`, `]`, and `?`. Since none of those characters appear
    /// anywhere in that function's body, there is no `if col.nullable { ...
    /// } else { ... }` (or any other conditional) capable of selecting a
    /// bracket/question-mark form in the first place -- not just that the
    /// current logic happens not to produce one. A future edit that tried to
    /// reintroduce optional-property syntax would have to touch those
    /// characters and would fail this test before it could reach a `cargo
    /// test` run that exercises the output.
    #[test]
    fn test_js_property_line_source_cannot_emit_optional_syntax_into_the_name_position() {
        let source = include_str!("typescript_common.rs");
        let start = source
            .find("// jsdoc-property-line-start")
            .expect("marker comment must exist in this file");
        let end = source
            .find("// jsdoc-property-line-end")
            .expect("marker comment must exist in this file");
        let body = &source[start..end];

        assert!(
            !body.contains('['),
            "js_property_line's source must contain no '[' -- a bracket-optional code path \
             would need one: {body}"
        );
        assert!(
            !body.contains(']'),
            "js_property_line's source must contain no ']' -- a bracket-optional code path \
             would need one: {body}"
        );
        assert!(
            !body.contains('?'),
            "js_property_line's source must contain no '?' -- a question-mark-optional code \
             path would need one: {body}"
        );
    }

    /// Runtime companion to the source-level guard above: for both a
    /// nullable and a non-nullable column, the emitted `@property` line
    /// carries the type only -- never a bracket or `?` around the name.
    #[test]
    fn test_js_property_line_never_emits_bracket_or_question_mark_syntax() {
        let nullable = column("bio", "string", true, None, false);
        let non_nullable = column("id", "number", false, None, false);

        let nullable_line = js_property_line(&nullable);
        let non_nullable_line = js_property_line(&non_nullable);

        assert_eq!(nullable_line, " * @property {string | null} bio");
        assert_eq!(non_nullable_line, " * @property {number} id");
        assert!(!nullable_line.contains('['), "{nullable_line}");
        assert!(!nullable_line.contains(']'), "{nullable_line}");
        assert!(!nullable_line.contains('?'), "{nullable_line}");
    }

    #[test]
    fn test_generate_js_typedef_row_struct_matches_property_lines() {
        let columns = vec![
            column("id", "number", false, None, false),
            column("bio", "string", true, None, false),
        ];
        let out = generate_js_typedef_row_struct("GetUserRow", "GetUser", &columns);

        assert!(out.starts_with("/**\n"), "{out}");
        assert!(out.contains(" * Row type for GetUser queries.\n"), "{out}");
        assert!(out.contains(" * @typedef {object} GetUserRow\n"), "{out}");
        assert!(out.contains(" * @property {number} id\n"), "{out}");
        assert!(out.contains(" * @property {string | null} bio\n"), "{out}");
        assert!(out.ends_with(" */"), "{out}");
    }

    #[test]
    fn test_js_type_cast_wraps_expr_in_a_jsdoc_type_comment() {
        assert_eq!(
            js_type_cast("GetUserRow | undefined", "stmt.get()"),
            "/** @type {GetUserRow | undefined} */ (stmt.get())"
        );
    }

    #[test]
    fn test_generate_jsdoc_fn_header_renders_params_and_return() {
        let out = generate_jsdoc_fn_header(
            "Fetch a single row.",
            &[
                ("client".to_string(), "import(\"pg\").PoolClient".to_string()),
                ("id".to_string(), "number".to_string()),
            ],
            "Promise<Row | null>",
        );

        assert_eq!(
            out,
            "/**\n * Fetch a single row.\n * @param {import(\"pg\").PoolClient} client\n * @param {number} id\n \
             * @returns {Promise<Row | null>}\n */"
        );
    }

    #[test]
    fn test_js_fn_signature_line_has_no_type_annotations() {
        let sig_params = vec![
            ("client".to_string(), "import(\"pg\").PoolClient".to_string()),
            ("id".to_string(), "number".to_string()),
        ];
        assert_eq!(
            js_fn_signature_line(true, "getUserById", &sig_params),
            "export async function getUserById(client, id) {"
        );
        assert_eq!(
            js_fn_signature_line(false, "getUserById", &sig_params),
            "export function getUserById(client, id) {"
        );
    }

    #[test]
    fn test_levenshtein_distance_basic_cases() {
        assert_eq!(levenshtein_distance("row_type", "row_type"), 0);
        assert_eq!(levenshtein_distance("row_typ", "row_type"), 1);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
    }
}
