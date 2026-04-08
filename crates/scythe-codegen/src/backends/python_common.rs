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
