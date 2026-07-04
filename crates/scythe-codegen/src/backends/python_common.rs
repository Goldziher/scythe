use scythe_backend::manifest::BackendManifest;
use scythe_core::errors::{ErrorCode, ScytheError};

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
    pub fn from_option(value: &str) -> Result<Self, ScytheError> {
        match value {
            "dataclass" => Ok(Self::Dataclass),
            "pydantic" => Ok(Self::Pydantic),
            "msgspec" => Ok(Self::Msgspec),
            _ => Err(ScytheError::new(
                ErrorCode::InternalError,
                format!(
                    "invalid row_type '{}': expected 'dataclass', 'pydantic', or 'msgspec'",
                    value
                ),
            )),
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
            // Both bare imports or both from imports: sort by module name.
            (true, true) | (false, false) => {
                if row_import < library_import {
                    format!("{row_import}\n{library_import}")
                } else {
                    format!("{library_import}\n{row_import}")
                }
            }
            // Row is bare, library is from: bare comes first.
            (true, false) => format!("{row_import}\n{library_import}"),
            // Row is from, library is bare: bare comes first.
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

    // Child class first — parent references it in the `children` field.
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

    // Parent class — all parent columns plus a `children` list field.
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
