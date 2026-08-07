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
    /// Struct definitions needed for nested-aggregate result shapes
    /// (`json_agg(o.*)`, `row_to_json(u.*)`, ...). PostgreSQL only; always
    /// empty for every other dialect.
    ///
    /// `#[serde(default)]` so payloads serialized before this field existed
    /// keep deserializing — but adding this key changes any content hash
    /// computed over the serialized `AnalyzedQuery`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub nested_structs: Vec<NestedStructInfo>,
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

/// A struct definition synthesized for a nested-aggregate result column
/// (`json_agg(o.*)`, `row_to_json(u.*)`, ...). PostgreSQL only.
///
/// Distinct from [`CompositeInfo`]/[`CompositeFieldInfo`]: those describe a
/// SQL composite *type* from the catalog, where every backend's
/// `generate_composite_def` hardcodes `nullable: false` on every field (no
/// per-field nullability is tracked). A nested-aggregate field's
/// nullability instead comes from the source column it was built from, so
/// [`NestedFieldInfo`] carries a real `nullable` rather than reusing that
/// channel.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NestedStructInfo {
    /// snake_case name, e.g. `get_user_orders_row_orders` — survives each
    /// backend's own `to_pascal_case` step, unlike the PascalCase form
    /// embedded in the owning column's `neutral_type`.
    pub name: String,
    pub fields: Vec<NestedFieldInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NestedFieldInfo {
    pub name: String,
    pub neutral_type: String,
    pub nullable: bool,
}

/// A nested-struct definition captured during expression inference (phase
/// 1 — `Analyzer::infer_nested_aggregate_type` in `expressions.rs`, the
/// `json_agg`/`row_to_json` producer built on
/// [`crate::analyzer::expressions::FuncArgShape`]), before the query name
/// and output column alias are known and therefore before the struct can
/// be named.
///
/// `analyze()`'s phase-2 pass resolves each entry into a [`NestedStructInfo`]
/// once `columns` (with aliases applied) exist, and replaces the
/// `__nested__{id}` placeholder embedded in the owning column's
/// `neutral_type` with the resolved name.
///
/// `id` is producer-assigned and must be unique within one `analyze()` call.
/// `build_scope_from_from`'s derived-subquery path bubbles a sub-analyzer's
/// `pending_nested` up to its parent (mirroring how it already bubbles
/// `params`/`type_errors`/`positional_param_counter`); a future producer
/// that assigns ids from a plain per-`Analyzer` counter must thread that
/// counter through the sub-analyzer the same way `positional_param_counter`
/// already is, or ids can collide across a subquery boundary.
#[derive(Debug, Clone)]
pub(super) struct PendingNestedStruct {
    pub(super) id: u32,
    pub(super) fields: Vec<NestedFieldInfo>,
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
    pub(super) fn from_type_info(name: impl Into<String>, type_info: TypeInfo) -> Self {
        // `sql_type` falls back to `neutral_type` for computed expressions (the
        // common case), so only clone when the two genuinely diverge — avoids an
        // extra allocation on every expression node in the tree.
        let sql_type = type_info.sql_type.unwrap_or_else(|| type_info.neutral_type.clone());
        Self {
            name: name.into(),
            neutral_type: type_info.neutral_type,
            nullable: type_info.nullable,
            join_group: type_info.join_group,
            nullable_before_join: type_info.nullable_before_join,
            sql_type,
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
    /// `None` means "same as `neutral_type`" — the common case for computed
    /// expressions — which avoids allocating a duplicate `String` on every
    /// expression node during type inference.
    pub(super) sql_type: Option<String>,
}

impl TypeInfo {
    pub(super) fn new(neutral_type: impl Into<String>, nullable: bool) -> Self {
        let neutral_type = neutral_type.into();
        Self {
            sql_type: None,
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
            sql_type: Some(sql_type.into()),
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
    /// Nested-struct definitions awaiting phase-2 naming. See
    /// [`PendingNestedStruct`].
    pub(super) pending_nested: Vec<PendingNestedStruct>,
    /// Auto-incrementing counter assigning `PendingNestedStruct::id`.
    /// Threaded into a derived-subquery sub-analyzer and back (mirroring
    /// `positional_param_counter`) so ids stay unique across a subquery
    /// boundary once `pending_nested` is bubbled up.
    pub(super) next_nested_id: u32,
}

impl<'a> Analyzer<'a> {
    /// Allocate the next `__nested__{id}` placeholder id and record its
    /// field shape for phase-2 naming to resolve.
    pub(super) fn push_pending_nested(&mut self, fields: Vec<NestedFieldInfo>) -> u32 {
        let id = self.next_nested_id;
        self.next_nested_id += 1;
        self.pending_nested.push(PendingNestedStruct { id, fields });
        id
    }
}
