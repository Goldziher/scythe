use crate::parser::{CustomAnnotation, QueryCommand};

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AnalyzedQuery {
    pub name: String,
    pub command: QueryCommand,
    pub sql: String,
    pub columns: Vec<AnalyzedColumn>,
    pub params: Vec<AnalyzedParam>,
    pub deprecated: Option<String>,
    /// If this is a SELECT * from a single table, the table name
    pub source_table: Option<String>,
    /// Composite type definitions needed by this query
    pub composites: Vec<CompositeInfo>,
    /// Enum type definitions needed by this query
    pub enums: Vec<EnumInfo>,
    /// Parameter names marked @optional — triggers SQL rewriting in codegen
    pub optional_params: Vec<String>,
    /// Grouping configuration for :grouped queries
    pub group_by: Option<GroupByConfig>,
    /// Custom (non-native) annotations from the SQL source, in source order.
    /// See [`CustomAnnotation`] for usage.
    pub custom: Vec<CustomAnnotation>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GroupByConfig {
    /// The table (or alias) used as the grouping parent, e.g. "users"
    pub table: String,
    /// The key column within the parent table, e.g. "id"
    pub key_column: String,
    /// Columns belonging to the parent table
    pub parent_columns: Vec<AnalyzedColumn>,
    /// Columns belonging to child table(s)
    pub child_columns: Vec<AnalyzedColumn>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CompositeInfo {
    pub sql_name: String,
    pub fields: Vec<CompositeFieldInfo>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CompositeFieldInfo {
    pub name: String,
    pub neutral_type: String,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EnumInfo {
    pub sql_name: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AnalyzedColumn {
    pub name: String,
    pub neutral_type: String,
    pub nullable: bool,
    /// Alias of the outer-joined relation this column came from, when the
    /// column was widened to nullable by that join.
    ///
    /// Columns sharing a group go null together, so a target language that can
    /// express it may emit one discriminated union rather than independent
    /// per-column optionals.
    #[cfg_attr(feature = "serde", serde(default))]
    pub join_group: Option<String>,
    /// Whether the column was nullable in the schema, before outer-join
    /// widening.  A column in a `join_group` with this `false` can only be null
    /// when the join found no matching row.
    #[cfg_attr(feature = "serde", serde(default))]
    pub nullable_before_join: bool,
    /// The raw (lowercased, precision-stripped) SQL type this column was derived
    /// from, e.g. "clob" or "varchar2". Backends that need to distinguish
    /// storage representations the neutral type collapses (Oracle CLOB vs.
    /// VARCHAR2, both `neutral_type == "string"`) can match on this. Falls back
    /// to `neutral_type` for computed/expression columns with no single source
    /// SQL type.
    #[cfg_attr(feature = "serde", serde(default))]
    pub sql_type: String,
}

impl AnalyzedColumn {
    /// Build a result column from an inferred expression type, carrying the
    /// outer-join provenance and source SQL type through.
    ///
    /// Aliasing a column must not lose its group — `o.total AS order_total` is
    /// still decided by the same join outcome as its siblings.
    pub(super) fn from_type_info(name: impl Into<String>, type_info: &TypeInfo) -> Self {
        Self {
            name: name.into(),
            neutral_type: type_info.neutral_type.clone(),
            nullable: type_info.nullable,
            join_group: type_info.join_group.clone(),
            nullable_before_join: type_info.nullable_before_join,
            sql_type: type_info.sql_type.clone(),
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AnalyzedParam {
    pub name: String,
    pub neutral_type: String,
    pub nullable: bool,
    pub position: i64,
}

#[derive(Debug, Clone)]
pub(super) struct ScopeSource {
    pub(super) alias: String,
    pub(super) table_name: String,
    pub(super) columns: Vec<ScopeColumn>,
    pub(super) nullable_from_join: bool,
}

#[derive(Debug, Clone)]
pub(super) struct ScopeColumn {
    pub(super) name: String,
    pub(super) sql_type: String,
    pub(super) neutral_type: String,
    pub(super) base_nullable: bool,
}

impl ScopeColumn {
    /// Build a scope column with no richer source-type information than its
    /// neutral type (synthetic columns: JSON table functions, aliased
    /// function-table outputs, literal/CTE placeholders, etc).
    pub(super) fn new(name: impl Into<String>, neutral_type: impl Into<String>, base_nullable: bool) -> Self {
        let neutral_type = neutral_type.into();
        Self {
            name: name.into(),
            sql_type: neutral_type.clone(),
            neutral_type,
            base_nullable,
        }
    }

    /// Build a scope column backed by a real catalog (or propagated
    /// upstream-analyzed) column, preserving its raw SQL type alongside the
    /// derived neutral type.
    pub(super) fn from_catalog(
        name: impl Into<String>,
        sql_type: impl Into<String>,
        neutral_type: impl Into<String>,
        base_nullable: bool,
    ) -> Self {
        Self {
            name: name.into(),
            sql_type: sql_type.into(),
            neutral_type: neutral_type.into(),
            base_nullable,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Scope {
    pub(super) sources: Vec<ScopeSource>,
}

/// Tracked parameter info during analysis
#[derive(Debug, Clone)]
pub(super) struct ParamInfo {
    pub(super) position: i64,
    pub(super) name: Option<String>,
    pub(super) neutral_type: Option<String>,
    pub(super) nullable: bool,
}

/// Result of inferring an expression's type
#[derive(Debug, Clone, Default)]
pub(super) struct TypeInfo {
    pub(super) neutral_type: String,
    pub(super) nullable: bool,
    /// Alias of the outer-joined relation this value came from, when it was
    /// widened to nullable by an outer join.  `None` for anything else.
    ///
    /// Columns sharing a `join_group` become null together — they are decided
    /// by the same match/no-match outcome — which is what lets a target
    /// language express them as one discriminated union instead of independent
    /// optionals.
    pub(super) join_group: Option<String>,
    /// Whether the value was already nullable in the schema, before any
    /// outer-join widening.
    ///
    /// A column in a `join_group` with `nullable_before_join == false` is a
    /// *discriminant*: it can only be null when the join found no row.
    pub(super) nullable_before_join: bool,
    /// Raw SQL type this expression's type came from, when it resolves
    /// directly to a single source column (see [`AnalyzedColumn::sql_type`]).
    /// Defaults to a copy of `neutral_type` for computed expressions.
    pub(super) sql_type: String,
}

impl TypeInfo {
    pub(super) fn new(neutral_type: impl Into<String>, nullable: bool) -> Self {
        let neutral_type = neutral_type.into();
        Self {
            sql_type: neutral_type.clone(),
            neutral_type,
            nullable,
            join_group: None,
            nullable_before_join: nullable,
        }
    }

    /// Build type info for a column resolved from a relation in scope.
    ///
    /// Carries two things the neutral type alone cannot express: the raw SQL
    /// type, so backends can distinguish storage representations that collapse
    /// to the same neutral type (Oracle CLOB vs. VARCHAR2), and where the
    /// column's nullability came from, so outer-joined columns can be grouped.
    pub(super) fn from_scope_column(
        sql_type: impl Into<String>,
        neutral_type: impl Into<String>,
        base_nullable: bool,
        source_alias: &str,
        nullable_from_join: bool,
    ) -> Self {
        Self {
            sql_type: sql_type.into(),
            neutral_type: neutral_type.into(),
            nullable: base_nullable || nullable_from_join,
            join_group: nullable_from_join.then(|| source_alias.to_string()),
            nullable_before_join: base_nullable,
        }
    }

    pub(super) fn unknown() -> Self {
        Self::new("unknown", true)
    }
}

use ahash::AHashMap;

use crate::catalog::Catalog;

pub(super) struct Analyzer<'a> {
    pub(super) catalog: &'a Catalog,
    pub(super) params: Vec<ParamInfo>,
    /// CTE name -> columns
    pub(super) ctes: AHashMap<String, Vec<ScopeColumn>>,
    /// Collected type errors during analysis
    pub(super) type_errors: Vec<String>,
    /// Auto-incrementing counter for MySQL `?` positional placeholders
    pub(super) positional_param_counter: i64,
}
