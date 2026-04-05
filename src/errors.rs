use std::fmt;

#[derive(Debug)]
pub struct ScytheError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    SyntaxError,
    UnknownTable,
    UnknownColumn,
    UnknownFunction,
    AmbiguousColumn,
    TypeMismatch,
    MissingAnnotation,
    InvalidAnnotation,
    ColumnCountMismatch,
    DuplicateAlias,
    InvalidRecursion,
    InternalError,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorCode::SyntaxError => write!(f, "SYNTAX_ERROR"),
            ErrorCode::UnknownTable => write!(f, "UNKNOWN_TABLE"),
            ErrorCode::UnknownColumn => write!(f, "UNKNOWN_COLUMN"),
            ErrorCode::UnknownFunction => write!(f, "UNKNOWN_FUNCTION"),
            ErrorCode::AmbiguousColumn => write!(f, "AMBIGUOUS_COLUMN"),
            ErrorCode::TypeMismatch => write!(f, "TYPE_MISMATCH"),
            ErrorCode::MissingAnnotation => write!(f, "MISSING_ANNOTATION"),
            ErrorCode::InvalidAnnotation => write!(f, "INVALID_ANNOTATION"),
            ErrorCode::ColumnCountMismatch => write!(f, "COLUMN_COUNT_MISMATCH"),
            ErrorCode::DuplicateAlias => write!(f, "DUPLICATE_ALIAS"),
            ErrorCode::InvalidRecursion => write!(f, "INVALID_RECURSION"),
            ErrorCode::InternalError => write!(f, "INTERNAL_ERROR"),
        }
    }
}

impl fmt::Display for ScytheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ScytheError {}

impl ScytheError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn syntax(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::SyntaxError, msg)
    }

    pub fn unknown_table(name: &str) -> Self {
        Self::new(
            ErrorCode::UnknownTable,
            format!("table \"{name}\" does not exist"),
        )
    }

    pub fn unknown_column(name: &str) -> Self {
        Self::new(
            ErrorCode::UnknownColumn,
            format!("column \"{name}\" does not exist"),
        )
    }

    pub fn unknown_function(name: &str) -> Self {
        Self::new(
            ErrorCode::UnknownFunction,
            format!("function \"{name}\" does not exist"),
        )
    }

    pub fn ambiguous_column(name: &str) -> Self {
        Self::new(
            ErrorCode::AmbiguousColumn,
            format!("column \"{name}\" is ambiguous"),
        )
    }

    pub fn type_mismatch(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::TypeMismatch, msg)
    }

    pub fn missing_annotation(what: &str) -> Self {
        Self::new(
            ErrorCode::MissingAnnotation,
            format!("missing @{what} annotation"),
        )
    }

    pub fn invalid_annotation(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidAnnotation, msg)
    }

    pub fn column_count_mismatch(left: usize, right: usize) -> Self {
        Self::new(
            ErrorCode::ColumnCountMismatch,
            format!("column count mismatch: {left} vs {right}"),
        )
    }

    pub fn duplicate_alias(name: &str) -> Self {
        Self::new(
            ErrorCode::DuplicateAlias,
            format!("duplicate column alias \"{name}\""),
        )
    }

    pub fn invalid_recursion(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRecursion, msg)
    }
}
