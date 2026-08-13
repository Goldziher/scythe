use scythe_backend::manifest::BackendManifest;
use scythe_core::errors::ScytheError;

use std::fmt::Write as _;

use crate::backend_trait::ResolvedColumn;

/// Supported Python row type styles for generated code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PythonRowType {
    #[default]
    Dataclass,
    Pydantic,
    Msgspec,
}

impl PythonRowType {
    /// Parse a row_type option string into a `PythonRowType`.
    ///
    /// A bad value here came from the user's `scythe.toml`, not from scythe's
    /// own state, so it is classified `InvalidConfig` (not `InternalError`)
    /// -- see [`ScytheError::invalid_config`].
    pub fn from_option(value: &str) -> Result<Self, ScytheError> {
        match value {
            "dataclass" => Ok(Self::Dataclass),
            "pydantic" => Ok(Self::Pydantic),
            "msgspec" => Ok(Self::Msgspec),
            _ => Err(ScytheError::invalid_config(format!(
                "invalid row_type '{}': expected 'dataclass', 'pydantic', or 'msgspec'",
                value
            ))),
        }
    }

    /// Returns the import line for the row type.
    pub fn import_line(self) -> &'static str {
        match self {
            Self::Dataclass => "from dataclasses import dataclass",
            Self::Pydantic => "from pydantic import BaseModel",
            Self::Msgspec => "import msgspec",
        }
    }

    /// Whether the row type import is a stdlib import (vs third-party).
    pub fn is_stdlib_import(self) -> bool {
        matches!(self, Self::Dataclass)
    }

    /// Build a sorted third-party import block combining the row type import
    /// with the given library import line.
    ///
    /// isort rules: bare `import` statements come before `from` statements,
    /// both groups sorted by module name.
    pub fn sorted_third_party_imports(self, library_import: &str) -> String {
        let row_import = self.import_line();
        let row_is_bare = row_import.starts_with("import ");
        let lib_is_bare = library_import.starts_with("import ");

        match (row_is_bare, lib_is_bare) {
            (true, true) | (false, false) => {
                if row_import < library_import {
                    format!("{row_import}\n{library_import}")
                } else {
                    format!("{library_import}\n{row_import}")
                }
            }
            (true, false) => format!("{row_import}\n{library_import}"),
            (false, true) => format!("{library_import}\n{row_import}"),
        }
    }

    /// Returns the decorator line (for dataclass) or empty string (for others).
    pub fn decorator(self) -> &'static str {
        match self {
            Self::Dataclass => "@dataclass(frozen=True, slots=True)\n",
            Self::Pydantic | Self::Msgspec => "",
        }
    }

    /// Returns the class definition line including the class name.
    pub fn class_def(self, class_name: &str) -> String {
        match self {
            Self::Dataclass => format!("class {}:", class_name),
            Self::Pydantic => format!("class {}(BaseModel):", class_name),
            Self::Msgspec => format!("class {}(msgspec.Struct):", class_name),
        }
    }
}

/// Returns `(needs_uuid, needs_any)`: whether the manifest's scalar type mappings reference
/// `uuid.UUID` or `Any` (i.e. `dict[str, Any]`), indicating which stdlib imports the generated
/// file header must emit to avoid a `NameError` at import time.
///
/// Mirrors the always-present `datetime`/`decimal` imports but emits only when actually needed,
/// following the kotlin-jdbc uuid-import precedent.
pub fn type_support_imports(manifest: &BackendManifest) -> (bool, bool) {
    let mut needs_uuid = false;
    let mut needs_any = false;
    for value in manifest.types.scalars.values() {
        if value.contains("uuid.UUID") {
            needs_uuid = true;
        }
        if value.contains("Any") {
            needs_any = true;
        }
    }
    (needs_uuid, needs_any)
}

/// Emit a DB-API `.execute(...)` (or `.executemany(...)`) call, wrapping it across
/// multiple lines when the single-line form would exceed 88 characters (ruff's default
/// line length / `E501`) — mirroring the single-line/multi-line switch already used for
/// `return` statements in the pyodbc and Snowflake backends.
///
/// `indent` is the leading whitespace for the statement (e.g. `"    "` or `"        "`
/// for a call nested inside `async with`). `call_expr` is everything before the opening
/// paren, e.g. `"cur.execute"` or `"await cur.executemany"`. `args`, when present, is the
/// second positional argument passed to `execute` (the bound parameters).
pub fn write_execute_call(out: &mut String, indent: &str, call_expr: &str, sql: &str, args: Option<&str>) {
    let sql = crate::sql_literal::escape_python_triple_double(sql);
    let oneliner = match args {
        Some(args) => format!("{indent}{call_expr}(\"\"\"{sql}\"\"\", {args})"),
        None => format!("{indent}{call_expr}(\"\"\"{sql}\"\"\")"),
    };
    if oneliner.len() <= 88 {
        let _ = writeln!(out, "{oneliner}");
    } else {
        let _ = writeln!(out, "{indent}{call_expr}(");
        let _ = writeln!(out, "{indent}    \"\"\"{sql}\"\"\",");
        if let Some(args) = args {
            let _ = writeln!(out, "{indent}    {args},");
        }
        let _ = writeln!(out, "{indent})");
    }
}

/// Emit a `def`/`async def` query-function signature, wrapping the parameter list across
/// multiple lines when the single-line form would exceed 88 characters (ruff's default line
/// length / `E501`) — the same threshold already used for `return` statements and
/// [`write_execute_call`] in these backends. Needed because some drivers' connection types
/// are long enough on their own (e.g. `snowflake.connector.SnowflakeConnection`) to push an
/// otherwise-short signature over the limit even with a single keyword parameter.
///
/// `conn_param` is the full first parameter, e.g. `"conn: oracledb.AsyncConnection"`.
/// `kw_params` become keyword-only parameters (after a `*,` separator) as `(name, type)`
/// pairs. `ret` is the return type annotation, e.g. `"GetUserRow | None"`.
pub fn write_def_signature(
    out: &mut String,
    def_kw: &str,
    func_name: &str,
    conn_param: &str,
    kw_params: &[(String, String)],
    ret: &str,
) {
    let param_list = kw_params
        .iter()
        .map(|(n, t)| format!("{n}: {t}"))
        .collect::<Vec<_>>()
        .join(", ");
    let kw_sep = if kw_params.is_empty() { "" } else { ", *, " };
    let oneliner = format!("{def_kw} {func_name}({conn_param}{kw_sep}{param_list}) -> {ret}:");
    if oneliner.len() <= 88 {
        let _ = writeln!(out, "{oneliner}");
    } else {
        let _ = writeln!(out, "{def_kw} {func_name}(");
        let _ = writeln!(out, "    {conn_param},");
        if !kw_params.is_empty() {
            let _ = writeln!(out, "    *,");
            for (n, t) in kw_params {
                let _ = writeln!(out, "    {n}: {t},");
            }
        }
        let _ = writeln!(out, ") -> {ret}:");
    }
}

/// Emit a `return StructName(field=value, ...)` statement, wrapping the constructor call
/// across multiple lines when the single-line form would exceed 88 characters. Mirrors the
/// inline `oneliner.len() <= 88` switch already used by the psycopg3, pyodbc, and Snowflake
/// backends for the same purpose; factored out so backends with different indentation (e.g.
/// oracledb, whose statements sit inside `async with conn.cursor() as cur:`) can share it.
pub fn write_return_call(out: &mut String, indent: &str, struct_name: &str, field_assignments: &[String]) {
    let oneliner = format!("{indent}return {struct_name}({})", field_assignments.join(", "));
    if oneliner.len() <= 88 {
        let _ = writeln!(out, "{oneliner}");
    } else {
        let _ = writeln!(out, "{indent}return {struct_name}(");
        for fa in field_assignments {
            let _ = writeln!(out, "{indent}    {fa},");
        }
        let _ = writeln!(out, "{indent})");
    }
}

/// Generate child and parent Python classes for a `:grouped` query.
///
/// Emits the child class first (to satisfy forward-reference requirements for the
/// parent's `children: list[child]` field), then the parent class.
pub fn generate_grouped_structs_py(
    row_type: PythonRowType,
    parent_struct_name: &str,
    child_struct_name: &str,
    parent_columns: &[ResolvedColumn],
    child_columns: &[ResolvedColumn],
) -> String {
    let mut out = String::new();

    let _ = write!(out, "{}", row_type.decorator());
    let _ = writeln!(out, "{}", row_type.class_def(child_struct_name));
    let _ = writeln!(out, "    \"\"\"Child row type for grouped query.\"\"\"");
    if child_columns.is_empty() {
        let _ = writeln!(out, "    pass");
    } else {
        let _ = writeln!(out);
        for col in child_columns {
            let _ = writeln!(out, "    {}: {}", col.field_name, col.full_type);
        }
    }

    let _ = writeln!(out);

    let _ = write!(out, "{}", row_type.decorator());
    let _ = writeln!(out, "{}", row_type.class_def(parent_struct_name));
    let _ = writeln!(out, "    \"\"\"Parent row type for grouped query.\"\"\"");
    let _ = writeln!(out);
    for col in parent_columns {
        let _ = writeln!(out, "    {}: {}", col.field_name, col.full_type);
    }
    let _ = writeln!(out, "    children: list[{child_struct_name}]");

    out
}

/// Name of the exception every generated python module raises from a `:one`
/// query when its row is missing. DB-API 2.0 has no built-in "no rows found"
/// exception -- unlike Go's `sql.ErrNoRows` or a Rust driver's own `Result`,
/// `fetchone()`/`fetchrow()` just hands back `None` -- so scythe defines this
/// class once per generated module (see [`no_rows_exception_def`]) and every
/// python backend's `:one` arm raises it under this shared name.
pub const NO_ROWS_EXCEPTION_NAME: &str = "ScytheNoRowsError";

/// The `ScytheNoRowsError` class definition, meant to be appended once to a
/// generated module's `file_header()` output. See [`NO_ROWS_EXCEPTION_NAME`].
///
/// Opens with a newline and ends with a blank line. The leading one is
/// load-bearing: every `file_header()` ends its import block with a single
/// trailing blank line, and ruff's `I001` counts the blank lines *after* an
/// import block as part of that block's formatting. Appending the class
/// directly would leave one blank line where PEP 8 and isort want two, so
/// every generated python module would fail `poly lint` on its own header.
/// Caught by `scythe generate --validate-output` (board #187) the first time
/// it ran.
pub fn no_rows_exception_def() -> String {
    format!(
        "\nclass {NO_ROWS_EXCEPTION_NAME}(Exception):\n    \"\"\"Raised by a `:one` query when no row matches.\"\"\"\n\n"
    )
}

/// Emit the `if {var} is None: ...` guard following a `:one`/`:opt` fetch:
/// `:one` (`is_one == true`) raises [`NO_ROWS_EXCEPTION_NAME`]; `:opt`
/// returns `None`. `indent` is the leading whitespace shared by both emitted
/// lines; `query_name` is `analyzed.name`, used in the raised message.
pub fn write_missing_row_guard(out: &mut String, indent: &str, var: &str, is_one: bool, query_name: &str) {
    let _ = writeln!(out, "{indent}if {var} is None:");
    if is_one {
        let _ = writeln!(
            out,
            "{indent}    raise {NO_ROWS_EXCEPTION_NAME}(\"{query_name}: no rows returned\")"
        );
    } else {
        let _ = writeln!(out, "{indent}    return None");
    }
}

/// Emit the client-side fold logic for a `:grouped` query that uses positional
/// (index-based) row access — all Python backends except asyncpg.
///
/// Assumes `rows` is already bound as a list of tuples in the calling function body.
/// Writes into `out`; ends WITHOUT a trailing newline (the caller may close out).
///
/// The fold is O(n) over rows: an insertion-order list (`_entries`) paired with a
/// dict index (`_index`) for O(1) key lookup. Each entry holds a parent-kwargs dict
/// and a children list; both are unpacked into the parent struct at the end.
pub fn generate_grouped_fold_positional(
    out: &mut String,
    all_columns: &[ResolvedColumn],
    parent_struct_name: &str,
    child_struct_name: &str,
    parent_columns: &[ResolvedColumn],
    child_columns: &[ResolvedColumn],
    key_column: &str,
) {
    let key_idx = all_columns.iter().position(|c| c.name == key_column).unwrap_or(0);

    let _ = writeln!(out, "    _index: dict = {{}}");
    let _ = writeln!(out, "    _entries: list = []");
    let _ = writeln!(out, "    for row in rows:");
    let _ = writeln!(out, "        key = row[{key_idx}]");
    let _ = writeln!(out, "        if key not in _index:");
    let _ = writeln!(out, "            _index[key] = len(_entries)");
    let _ = writeln!(out, "            _entries.append((");
    let _ = writeln!(out, "                {{");
    for col in parent_columns {
        let col_idx = all_columns.iter().position(|c| c.name == col.name).unwrap_or(0);
        let _ = writeln!(out, "                    \"{}\": row[{col_idx}],", col.field_name);
    }
    let _ = writeln!(out, "                }},");
    let _ = writeln!(out, "                [],");
    let _ = writeln!(out, "            ))");
    let _ = writeln!(out, "        _entries[_index[key]][1].append({child_struct_name}(");
    for col in child_columns {
        let col_idx = all_columns.iter().position(|c| c.name == col.name).unwrap_or(0);
        let _ = writeln!(out, "            {}=row[{col_idx}],", col.field_name);
    }
    let _ = writeln!(out, "        ))");
    let _ = writeln!(out, "    return [");
    let _ = writeln!(out, "        {parent_struct_name}(**parent_kwargs, children=children)");
    let _ = writeln!(out, "        for parent_kwargs, children in _entries");
    let _ = write!(out, "    ]");
}
