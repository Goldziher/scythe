use scythe_core::errors::{ErrorCode, ScytheError};

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
            Self::Dataclass => "@dataclass\n",
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
