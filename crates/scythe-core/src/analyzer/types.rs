use crate::parser::{CustomAnnotation, QueryCommand};

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

/// Everything inference established about one annotated query.
///
/// `#[non_exhaustive]`: adding a public field here is a breaking change for
/// every downstream struct literal — which is exactly what adding
/// `nested_structs` was. 0.14.0 is a breaking release regardless, so the
/// marker goes on now and the *next* field costs nothing. Build one with
/// [`AnalyzedQuery::build`]; `#[non_exhaustive]` rejects a struct literal
/// from another crate, including the `..Default::default()` form.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
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
    /// (`json_agg(o.*)`, `row_to_json(u.*)`, ...).
    ///
    /// Always empty unless the catalog is both PostgreSQL-dialect *and* on
    /// an engine that actually has those functions: Redshift and DuckDB map
    /// to `SqlDialect::PostgreSQL` but ship neither, so the dialect alone is
    /// not the gate. See `expressions::catalog_has_nested_aggregates`.
    ///
    /// `#[serde(default)]` so payloads serialized before this field existed
    /// keep deserializing — but adding this key changes any content hash
    /// computed over the serialized `AnalyzedQuery`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub nested_structs: Vec<NestedStructInfo>,
}

impl AnalyzedQuery {
    /// Build an `AnalyzedQuery` from [`Default`], assigning fields inside
    /// `init`.
    ///
    /// This is the supported replacement for a struct literal now that the
    /// type is `#[non_exhaustive]`. Keeping construction a single expression
    /// (rather than `let mut q = AnalyzedQuery::default(); q.name = ...`)
    /// also keeps clippy's `field_reassign_with_default` quiet at ~100 call
    /// sites.
    ///
    /// ```
    /// use scythe_core::analyzer::AnalyzedQuery;
    /// use scythe_core::parser::QueryCommand;
    ///
    /// let query = AnalyzedQuery::build(|q| {
    ///     q.name = "GetUser".to_string();
    ///     q.command = QueryCommand::One;
    /// });
    /// assert_eq!(query.name, "GetUser");
    /// ```
    #[must_use]
    pub fn build(init: impl FnOnce(&mut Self)) -> Self {
        let mut query = Self::default();
        init(&mut query);
        query
    }
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
    /// The table (or CTE/derived-relation alias) this column resolves to directly, e.g.
    /// `"users"` for `SELECT id FROM users` or `SELECT u.id FROM users u`. `None` for a
    /// column with no single owning relation -- a computed expression, a literal, or a
    /// function result -- which is exactly the case a qualified `column = "table.col"`
    /// type override can never legitimately target.
    ///
    /// Populated for both `SELECT *` expansion and an explicit select list, so a type
    /// override's `table.column` match no longer depends on the query being `SELECT *`
    /// (see #189: it silently matched nothing for any other projection).
    ///
    /// `#[serde(default)]` so payloads serialized before this field existed keep
    /// deserializing -- but adding this key changes any content hash computed over the
    /// serialized `AnalyzedQuery`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub source_relation: Option<String>,
    /// Whether this column's projected expression is, after stripping only
    /// parentheses/unary sign, a bare `?`/`$N` placeholder or a literal `NULL`
    /// with no `CAST`, comparison, `COALESCE`, or other typed context to
    /// borrow a type from -- the shape `analyze()` rejects with
    /// [`crate::errors::ScytheError::type_mismatch`] rather than letting it
    /// reach codegen as `neutral_type == "unknown"` (issue tracked from
    /// GH #170's residual half).
    ///
    /// Set only by [`TypeInfo`]'s two `Expr::Value` arms for `NULL` and a bare
    /// placeholder (see `expressions.rs`); every other path that produces
    /// `neutral_type == "unknown"` -- `jsonb_each`'s record column included --
    /// goes through a different `TypeInfo` constructor and leaves this `false`.
    ///
    /// Survives two boundaries that would otherwise launder it back to
    /// `false` before `analyze()` ever sees it: a UNION arm's widened
    /// `AnalyzedColumn` (`analyze_set_expr`'s `SetOperation` arm in
    /// `statements.rs`) carries it forward only when *both* sides are
    /// untyped -- either side supplying a real type clears it, since that
    /// side genuinely resolved the column; and a derived table or CTE
    /// (`ScopeColumn::from_analyzed_column` -> `TypeInfo::from_scope_column`)
    /// carries it one scope level up unchanged, so a bare literal projected
    /// out of a subquery is still flagged at the outer `SELECT`.
    ///
    /// Internal bookkeeping consumed by `analyze()` before it returns --
    /// `#[serde(skip)]` keeps it out of the public wire format entirely rather
    /// than requiring every existing payload to gain a new key.
    ///
    /// ~keep `pub`, not `pub(crate)`: `AnalyzedColumn` is constructed by struct
    /// literal in other crates' tests, and a private field makes even
    /// `..Default::default()` an E0451 there -- struct update syntax still
    /// requires every field to be visible at the call site.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub untyped_literal: bool,
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
            source_relation: type_info.source_relation,
            untyped_literal: type_info.untyped_literal,
        }
    }
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AnalyzedParam {
    pub name: String,
    pub neutral_type: String,
    pub nullable: bool,
    pub position: i64,
    /// The table (or CTE/derived-relation alias) this parameter's bound value is compared
    /// against directly, e.g. `"users"` for `WHERE users.email = $1`. `None` when the
    /// parameter has no single owning column -- a literal comparison with no column context,
    /// a `BETWEEN`/`LIKE`/`IN`-list/array binding, a function argument, a `CASE` branch --
    /// which is exactly the case a qualified `column = "table.col"` type override can never
    /// legitimately target (#189's remainder: [`AnalyzedColumn`] got this in 99227e8e,
    /// `AnalyzedParam` did not).
    ///
    /// Populated only for a direct binary comparison (`try_bind_param_from_comparison`), the
    /// one shape where "the column this parameter is compared against" is unambiguous. Every
    /// other binding site keeps `None` deliberately rather than guessing at a relation the
    /// parameter doesn't have one true owner for.
    ///
    /// `#[serde(default)]` so payloads serialized before this field existed keep
    /// deserializing -- but adding this key changes any content hash computed over the
    /// serialized `AnalyzedQuery`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub source_relation: Option<String>,
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
    /// See [`AnalyzedColumn::untyped_literal`]. `false` for every constructor
    /// except [`ScopeColumn::from_analyzed_column`], which carries an
    /// already-analyzed output column's taint through a derived-table/CTE
    /// boundary. A genuine catalog column ([`ScopeColumn::from_catalog`]) and
    /// a function's synthetic result column ([`ScopeColumn::new`] -- e.g.
    /// `jsonb_each`'s `key`/`value`) never originate a bare literal, so both
    /// leave this `false`.
    pub(super) untyped_literal: bool,
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
            untyped_literal: false,
        }
    }

    /// Build a scope column backed by a real catalog column, preserving its
    /// raw SQL type alongside the derived neutral type.
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
            untyped_literal: false,
        }
    }

    /// Build a scope column from an already-analyzed output column, carrying
    /// its [`AnalyzedColumn::untyped_literal`] taint through a derived-table
    /// or CTE boundary -- so `SELECT tag FROM (SELECT NULL AS tag FROM t) sub`
    /// still sees the inner `tag` as an untyped literal one level up, instead
    /// of the taint silently resetting the moment the column crosses a scope
    /// boundary. Used everywhere a derived table's or CTE's output columns
    /// are folded back into a `ScopeColumn` list; a real catalog column never
    /// goes through this constructor.
    pub(super) fn from_analyzed_column(col: &AnalyzedColumn) -> Self {
        Self {
            name: col.name.clone(),
            sql_type: col.sql_type.clone(),
            neutral_type: col.neutral_type.clone(),
            base_nullable: col.nullable,
            untyped_literal: col.untyped_literal,
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
    /// See [`AnalyzedParam::source_relation`]; carried through unchanged into the public type.
    pub(super) source_relation: Option<String>,
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
    /// The relation (table, CTE, or derived-alias) this value resolves to
    /// directly. `None` for anything without a single owning relation —
    /// a computed expression, a literal, a function result. See
    /// [`AnalyzedColumn::source_relation`].
    pub(super) source_relation: Option<String>,
    /// See [`AnalyzedColumn::untyped_literal`]. Set only by the two
    /// `Expr::Value` arms that infer a bare `NULL`/placeholder with nothing to
    /// widen or cast against; every constructor here defaults it `false` so a
    /// typed wrapper (`CAST`, a comparison, `COALESCE`, `widen_type_info`, ...)
    /// clears it the moment it builds a fresh `TypeInfo` around the value.
    pub(super) untyped_literal: bool,
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
            source_relation: None,
            untyped_literal: false,
        }
    }

    /// Build type info for a column resolved from a relation in scope.
    ///
    /// Carries four things the neutral type alone cannot express: the raw SQL
    /// type, so backends can distinguish storage representations that collapse
    /// to the same neutral type (Oracle CLOB vs. VARCHAR2); where the column's
    /// nullability came from, so outer-joined columns can be grouped; the
    /// owning relation's real table name (not just its query alias), so a
    /// qualified `table.column` type override can bind to it (#189); and the
    /// caller-supplied `untyped_literal`, so a bare `NULL`/placeholder that was
    /// projected out of a derived table or CTE (see
    /// [`ScopeColumn::from_analyzed_column`]) is still flagged one scope level
    /// up rather than resetting to `false` the moment it crosses that boundary.
    pub(super) fn from_scope_column(
        sql_type: impl Into<String>,
        neutral_type: impl Into<String>,
        base_nullable: bool,
        source_alias: &str,
        nullable_from_join: bool,
        source_table: &str,
        untyped_literal: bool,
    ) -> Self {
        Self {
            sql_type: Some(sql_type.into()),
            neutral_type: neutral_type.into(),
            nullable: base_nullable || nullable_from_join,
            join_group: nullable_from_join.then(|| source_alias.to_string()),
            nullable_before_join: base_nullable,
            source_relation: Some(source_table.to_string()),
            untyped_literal,
        }
    }

    pub(super) fn unknown() -> Self {
        Self::new("unknown", true)
    }

    /// [`TypeInfo::unknown`] for the one shape `analyze()` treats as an error
    /// rather than a legitimate `"unknown"` (a set-returning function's record
    /// column, a UNION arm not yet widened): a bare `?`/`$N` placeholder or a
    /// literal `NULL`, projected with nothing to cast or compare it against.
    /// See [`AnalyzedColumn::untyped_literal`].
    pub(super) fn untyped_literal() -> Self {
        Self {
            untyped_literal: true,
            ..Self::unknown()
        }
    }
}

use ahash::AHashMap;
use sqlparser::tokenizer::Span;

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
    /// Memoizes the position already assigned to a `?` occurrence, keyed by
    /// that occurrence's source span. A single placeholder AST node is
    /// visited twice per projection expression -- once by
    /// `collect_params_from_where`, once by `infer_expr_type` -- and `?` has
    /// no explicit position to make that idempotent the way `$N` is, so
    /// without this memo the second visit mints a brand-new position and
    /// the query reports the wrong parameter count (#170). Keyed by
    /// [`Span`] rather than an occurrence index because `sqlparser` (see
    /// `Parser::try_with_sql` -> `Tokenizer::tokenize_with_location`)
    /// always tokenizes with real line/column locations, so every `?` in
    /// parsed SQL carries a distinct, stable span. `Span::empty()` --
    /// which only synthetic/test AST nodes carry, never output from
    /// `Parser::parse_sql` -- deliberately bypasses this memo (see
    /// `resolve_placeholder_position`) rather than being treated as a
    /// valid key, since every such node would otherwise collapse onto the
    /// same entry. Starts empty and is extended (not overwritten) across
    /// a derived-subquery sub-analyzer boundary, the same way
    /// `pending_nested` is: a subquery's placeholder spans live in a
    /// disjoint region of the source text, so merging can never collide.
    pub(super) resolved_placeholders: AHashMap<Span, i64>,
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
