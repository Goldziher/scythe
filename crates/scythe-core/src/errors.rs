use std::fmt;

#[derive(Debug)]
pub struct ScytheError {
    pub code: ErrorCode,
    pub message: String,
}

/// A machine-readable classification for a [`ScytheError`].
///
/// Marked `#[non_exhaustive]` so that adding a variant is not a breaking change
/// for downstream crates matching on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorCode {
    SyntaxError,
    UnknownTable,
    UnknownColumn,
    UnknownFunction,
    AmbiguousFunction,
    AmbiguousColumn,
    TypeMismatch,
    MissingAnnotation,
    InvalidAnnotation,
    ColumnCountMismatch,
    DuplicateAlias,
    InvalidRecursion,
    /// The user's configuration is wrong — an unknown `[[sql.gen]]` key, an
    /// unrecognized option value, a malformed manifest override. Distinct from
    /// [`ErrorCode::InternalError`], which means scythe itself is at fault.
    InvalidConfig,
    /// A construct the analyzer diagnosed correctly but that has no neutral-type
    /// representation scythe can hand to codegen: a set-returning function's
    /// anonymous `record` column referenced in select-list position
    /// (`json_each`, `json_populate_record`, ...), a multi-element `ROW(...)`/
    /// tuple constructor, or any other expression `infer_expr_type` genuinely
    /// cannot resolve. Distinct from [`ErrorCode::TypeMismatch`], which covers
    /// an expression that *could* have been typed but nothing in the query gave
    /// it one (a bare NULL/placeholder, both arms of a UNION untyped) — this
    /// code is for input scythe understood and rejected on purpose (#223),
    /// mirroring the existing one-code-per-marker-family pattern
    /// ([`ErrorCode::AmbiguousColumn`], [`ErrorCode::UnknownColumn`],
    /// [`ErrorCode::UnknownFunction`]) rather than folding into `TypeMismatch`.
    UnresolvedType,
    InternalError,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorCode::SyntaxError => write!(f, "SYNTAX_ERROR"),
            ErrorCode::UnknownTable => write!(f, "UNKNOWN_TABLE"),
            ErrorCode::UnknownColumn => write!(f, "UNKNOWN_COLUMN"),
            ErrorCode::UnknownFunction => write!(f, "UNKNOWN_FUNCTION"),
            ErrorCode::AmbiguousFunction => write!(f, "AMBIGUOUS_FUNCTION"),
            ErrorCode::AmbiguousColumn => write!(f, "AMBIGUOUS_COLUMN"),
            ErrorCode::TypeMismatch => write!(f, "TYPE_MISMATCH"),
            ErrorCode::MissingAnnotation => write!(f, "MISSING_ANNOTATION"),
            ErrorCode::InvalidAnnotation => write!(f, "INVALID_ANNOTATION"),
            ErrorCode::ColumnCountMismatch => write!(f, "COLUMN_COUNT_MISMATCH"),
            ErrorCode::DuplicateAlias => write!(f, "DUPLICATE_ALIAS"),
            ErrorCode::InvalidRecursion => write!(f, "INVALID_RECURSION"),
            ErrorCode::InvalidConfig => write!(f, "INVALID_CONFIG"),
            ErrorCode::UnresolvedType => write!(f, "UNRESOLVED_TYPE"),
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
        Self::new(ErrorCode::UnknownTable, format!("table \"{name}\" does not exist"))
    }

    pub fn unknown_column(name: &str) -> Self {
        Self::new(ErrorCode::UnknownColumn, format!("column \"{name}\" does not exist"))
    }

    pub fn unknown_function(name: &str) -> Self {
        Self::new(
            ErrorCode::UnknownFunction,
            format!("function \"{name}\" does not exist"),
        )
    }

    pub fn ambiguous_function(name: &str) -> Self {
        Self::new(
            ErrorCode::AmbiguousFunction,
            format!("function call \"{name}\" is ambiguous"),
        )
    }

    pub fn ambiguous_column(name: &str) -> Self {
        Self::new(ErrorCode::AmbiguousColumn, format!("column \"{name}\" is ambiguous"))
    }

    pub fn type_mismatch(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::TypeMismatch, msg)
    }

    pub fn missing_annotation(what: &str) -> Self {
        Self::new(ErrorCode::MissingAnnotation, format!("missing @{what} annotation"))
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
        Self::new(ErrorCode::DuplicateAlias, format!("duplicate column alias \"{name}\""))
    }

    /// The explicit column alias list on a CTE (`WITH t(a, b) AS ...`) has a
    /// different entry count than the CTE body's output columns. PostgreSQL
    /// rejects this at parse/plan time, so scythe must too — silently matching
    /// by position would mislabel columns.
    pub fn cte_column_alias_mismatch(alias_count: usize, column_count: usize) -> Self {
        Self::new(
            ErrorCode::ColumnCountMismatch,
            format!("CTE column alias list has {alias_count} entries but the CTE body produces {column_count} columns"),
        )
    }

    pub fn invalid_recursion(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRecursion, msg)
    }

    /// Build an error for user-supplied configuration that scythe understood
    /// well enough to reject — a misspelled option key, an out-of-range value.
    ///
    /// Prefer this over [`ScytheError::new`] with [`ErrorCode::InternalError`]
    /// anywhere the offending input came from `scythe.toml` rather than from
    /// scythe's own state.
    pub fn invalid_config(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidConfig, msg)
    }

    /// A set-returning function's anonymous `record` column referenced in
    /// select-list position -- `json_each`/`jsonb_each`/`json_each_text`/
    /// `jsonb_each_text` and the `json_populate_record`/`jsonb_populate_record`/
    /// `json_populate_recordset`/`jsonb_populate_recordset` family. PostgreSQL's
    /// `record` pseudo-type has no neutral-type representation, but every one
    /// of these functions has a FROM-clause form that expands into real,
    /// typeable columns instead (#223).
    ///
    /// The message deliberately does not hardcode `.key`/`.value` as the
    /// worked example: that's the right rewrite for the `json_each`/
    /// `json_each_text` family (see
    /// `testing_data/types/json_jsonb_advanced/06_jsonb_each_from.json`) but
    /// wrong for `json_populate_record`, whose columns come from a caller-supplied
    /// row type instead -- one message covering both families has to stay
    /// generic about the field names.
    pub fn untypeable_record(subject: &str, column: &str, function: &str) -> Self {
        Self::new(
            ErrorCode::UnresolvedType,
            format!(
                "{subject} \"{column}\": {function}(...) in the select list returns PostgreSQL's anonymous \
                 `record` type, which scythe cannot resolve to a type it can generate code for -- move it \
                 into the FROM clause instead (e.g. `FROM ..., {function}(...) AS {column}`) and select \
                 {column}'s fields directly instead of the whole record"
            ),
        )
    }

    /// A multi-element `ROW(...)`/tuple constructor -- `(a, b)` (parses as
    /// `Expr::Tuple`) or the explicit `ROW(a, b)` call (parses as a function
    /// named `row`) -- neither of which has a single neutral type (#117, #223).
    pub fn untypeable_row_constructor(subject: &str, column: &str) -> Self {
        Self::new(
            ErrorCode::UnresolvedType,
            format!(
                "{subject} \"{column}\": ROW(...) builds a row with no single neutral type scythe can name -- \
                 CAST it to a named composite type, or select its fields individually instead of one ROW(...) value"
            ),
        )
    }

    /// A generic fallback for any other expression `infer_expr_type` cannot
    /// resolve to a type: an unresolved composite field access, `unnest` over
    /// a non-array argument, a subquery producing no columns, a table-valued
    /// function scythe has no column-type mapping for, and the handful of AST
    /// shapes with no dedicated inference arm (#223).
    pub fn unresolved_expression_type(subject: &str, column: &str) -> Self {
        Self::new(
            ErrorCode::UnresolvedType,
            format!("{subject} \"{column}\": scythe could not determine a type for this expression"),
        )
    }
}
