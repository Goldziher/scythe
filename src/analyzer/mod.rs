use sqlparser::ast::{
    self, BinaryOperator, DataType, Expr, FunctionArg, FunctionArgExpr, JoinOperator, ObjectName,
    Query as SqlQuery, SelectItem, SetExpr, Statement, TableFactor, TableWithJoins, TimezoneInfo,
    UnaryOperator, Value,
};

use crate::catalog::Catalog;
use crate::errors::ScytheError;
use crate::parser::{Query, QueryCommand};

// ---------------------------------------------------------------------------
// Public output types
// ---------------------------------------------------------------------------

#[derive(Debug)]
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
}

#[derive(Debug, Clone)]
pub struct CompositeInfo {
    pub sql_name: String,
    pub fields: Vec<CompositeFieldInfo>,
}

#[derive(Debug, Clone)]
pub struct CompositeFieldInfo {
    pub name: String,
    pub neutral_type: String,
}

#[derive(Debug, Clone)]
pub struct EnumInfo {
    pub sql_name: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AnalyzedColumn {
    pub name: String,
    pub neutral_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone)]
pub struct AnalyzedParam {
    pub name: String,
    pub neutral_type: String,
    pub nullable: bool,
    pub position: i64,
}

// ---------------------------------------------------------------------------
// Internal scope types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ScopeSource {
    alias: String,
    table_name: String,
    columns: Vec<ScopeColumn>,
    nullable_from_join: bool,
}

#[derive(Debug, Clone)]
struct ScopeColumn {
    name: String,
    neutral_type: String,
    base_nullable: bool,
}

#[derive(Debug, Clone)]
struct Scope {
    sources: Vec<ScopeSource>,
}

/// Tracked parameter info during analysis
#[derive(Debug, Clone)]
struct ParamInfo {
    position: i64,
    name: Option<String>,
    neutral_type: Option<String>,
    nullable: bool,
}

/// Result of inferring an expression's type
#[derive(Debug, Clone)]
struct TypeInfo {
    neutral_type: String,
    nullable: bool,
}

impl TypeInfo {
    fn new(neutral_type: impl Into<String>, nullable: bool) -> Self {
        Self {
            neutral_type: neutral_type.into(),
            nullable,
        }
    }
    fn unknown() -> Self {
        Self::new("unknown", true)
    }
}

// ---------------------------------------------------------------------------
// Analyzer context
// ---------------------------------------------------------------------------

struct Analyzer<'a> {
    catalog: &'a Catalog,
    params: Vec<ParamInfo>,
    /// CTE name -> columns
    ctes: Vec<(String, Vec<ScopeColumn>)>,
    /// Collected type errors during analysis
    type_errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn analyze(catalog: &Catalog, query: &Query) -> Result<AnalyzedQuery, ScytheError> {
    let mut analyzer = Analyzer {
        catalog,
        params: Vec::new(),
        ctes: Vec::new(),
        type_errors: Vec::new(),
    };

    let (columns, _) = analyzer.analyze_statement(&query.stmt)?;

    // Check for type errors collected during analysis
    if let Some(err_msg) = analyzer.type_errors.first() {
        return Err(ScytheError::type_mismatch(err_msg.clone()));
    }

    // Apply annotation overrides
    let mut columns = columns;
    for col in &mut columns {
        if query
            .annotations
            .nullable_overrides
            .iter()
            .any(|o| o == &col.name)
        {
            col.nullable = true;
        }
        if query
            .annotations
            .nonnull_overrides
            .iter()
            .any(|o| o == &col.name)
        {
            col.nullable = false;
        }
        // Apply @json type mappings
        if let Some(mapping) = query
            .annotations
            .json_mappings
            .iter()
            .find(|m| m.column == col.name)
        {
            col.neutral_type = format!("json_typed<{}>", mapping.rust_type);
        }
    }

    // Deduplicate and sort params by position
    analyzer.params.sort_by_key(|p| p.position);
    analyzer.params.dedup_by_key(|p| p.position);

    let params: Vec<AnalyzedParam> = analyzer
        .params
        .iter()
        .map(|p| {
            let name = p.name.clone().unwrap_or_else(|| format!("p{}", p.position));
            let neutral_type = p
                .neutral_type
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            AnalyzedParam {
                name,
                neutral_type,
                nullable: p.nullable,
                position: p.position,
            }
        })
        .collect();

    // Detect SELECT * from single table for model struct reuse
    let source_table = detect_select_star_source(&query.stmt);

    // Collect composite type definitions needed
    let mut composites = Vec::new();
    let mut seen_composites: Vec<String> = Vec::new();
    for col in &columns {
        if let Some(comp_name) = col.neutral_type.strip_prefix("composite::")
            && !seen_composites.contains(&comp_name.to_string())
            && let Some(comp) = catalog.get_composite(comp_name)
        {
            composites.push(CompositeInfo {
                sql_name: comp_name.to_string(),
                fields: comp
                    .fields
                    .iter()
                    .map(|f| CompositeFieldInfo {
                        name: f.name.clone(),
                        neutral_type: sql_type_to_neutral(&f.sql_type, catalog),
                    })
                    .collect(),
            });
            seen_composites.push(comp_name.to_string());
        }
    }

    // Collect enum type definitions needed
    let mut enums = Vec::new();
    let mut seen_enums: Vec<String> = Vec::new();
    let all_types: Vec<&str> = columns
        .iter()
        .map(|c| c.neutral_type.as_str())
        .chain(params.iter().map(|p| p.neutral_type.as_str()))
        .collect();
    for nt in &all_types {
        if let Some(enum_name) = nt.strip_prefix("enum::")
            && !seen_enums.contains(&enum_name.to_string())
            && let Some(enum_type) = catalog.get_enum(enum_name)
        {
            enums.push(EnumInfo {
                sql_name: enum_name.to_string(),
                values: enum_type.values.clone(),
            });
            seen_enums.push(enum_name.to_string());
        }
    }

    Ok(AnalyzedQuery {
        name: query.name.clone(),
        command: query.command.clone(),
        sql: query.sql.clone(),
        columns,
        params,
        deprecated: query.annotations.deprecated.clone(),
        source_table,
        composites,
        enums,
    })
}

// ---------------------------------------------------------------------------
// SQL type -> neutral type mapping
// ---------------------------------------------------------------------------

fn sql_type_to_neutral(sql_type: &str, catalog: &Catalog) -> String {
    let lower = sql_type.to_lowercase();
    // Strip precision suffixes like "timestamp with time zone(6)"
    let normalized = strip_precision(&lower);

    match normalized.as_str() {
        "integer" | "int" | "int4" | "serial" => "int32".to_string(),
        "smallint" | "int2" | "smallserial" => "int16".to_string(),
        "bigint" | "int8" | "bigserial" => "int64".to_string(),
        "real" | "float4" => "float32".to_string(),
        "double precision" | "float8" => "float64".to_string(),
        "numeric" | "decimal" => "decimal".to_string(),
        "text" | "character varying" | "character" | "varchar" | "char" => "string".to_string(),
        "boolean" | "bool" => "bool".to_string(),
        "bytea" => "bytes".to_string(),
        "uuid" => "uuid".to_string(),
        "date" => "date".to_string(),
        "time" | "time without time zone" => "time".to_string(),
        "time with time zone" | "timetz" => "time_tz".to_string(),
        "timestamp" | "timestamp without time zone" => "datetime".to_string(),
        "timestamp with time zone" | "timestamptz" => "datetime_tz".to_string(),
        "interval" => "interval".to_string(),
        "json" | "jsonb" => "json".to_string(),
        "inet" | "cidr" | "macaddr" => "inet".to_string(),
        "integer[]" | "int4[]" | "int[]" => "array<int32>".to_string(),
        "text[]" | "character varying[]" | "varchar[]" => "array<string>".to_string(),
        "boolean[]" | "bool[]" => "array<bool>".to_string(),
        "bigint[]" | "int8[]" => "array<int64>".to_string(),
        "smallint[]" | "int2[]" => "array<int16>".to_string(),
        "real[]" | "float4[]" => "array<float32>".to_string(),
        "double precision[]" | "float8[]" => "array<float64>".to_string(),
        "uuid[]" => "array<uuid>".to_string(),
        "numeric[]" | "decimal[]" => "array<decimal>".to_string(),
        "jsonb[]" | "json[]" => "array<json>".to_string(),
        "int4range" => "range<int32>".to_string(),
        "int8range" => "range<int64>".to_string(),
        "tstzrange" => "range<datetime_tz>".to_string(),
        "tsrange" => "range<datetime>".to_string(),
        "daterange" => "range<date>".to_string(),
        "numrange" => "range<decimal>".to_string(),
        _ => {
            // Check for array types with brackets
            if let Some(inner) = normalized.strip_suffix("[]") {
                let inner_neutral = sql_type_to_neutral(inner, catalog);
                return format!("array<{}>", inner_neutral);
            }
            // Check enums
            if catalog.get_enum(&normalized).is_some() {
                return format!("enum::{}", normalized);
            }
            // Check composites
            if catalog.get_composite(&normalized).is_some() {
                return format!("composite::{}", normalized);
            }
            // Unknown type - return as-is
            normalized.to_string()
        }
    }
}

fn strip_precision(s: &str) -> String {
    // Remove trailing "(N)" from type names like "timestamp with time zone(6)"
    if let Some(idx) = s.rfind('(')
        && s.ends_with(')')
    {
        let prefix = s[..idx].trim();
        let inner = &s[idx + 1..s.len() - 1];
        if inner
            .chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c == ' ')
        {
            return prefix.to_string();
        }
    }
    s.to_string()
}

fn datatype_to_neutral(dt: &DataType, catalog: &Catalog) -> String {
    match dt {
        DataType::Int(_) | DataType::Int4(_) | DataType::Integer(_) => "int32".to_string(),
        DataType::SmallInt(_) | DataType::Int2(_) => "int16".to_string(),
        DataType::BigInt(_) | DataType::Int8(_) => "int64".to_string(),
        DataType::Real | DataType::Float4 => "float32".to_string(),
        DataType::DoublePrecision | DataType::Float8 => "float64".to_string(),
        DataType::Float(info) => {
            use sqlparser::ast::ExactNumberInfo;
            match info {
                ExactNumberInfo::Precision(p) if *p <= 24 => "float32".to_string(),
                _ => "float64".to_string(),
            }
        }
        DataType::Numeric(_) | DataType::Decimal(_) | DataType::Dec(_) => "decimal".to_string(),
        DataType::Varchar(_)
        | DataType::CharVarying(_)
        | DataType::CharacterVarying(_)
        | DataType::Text
        | DataType::Char(_)
        | DataType::Character(_) => "string".to_string(),
        DataType::Bool | DataType::Boolean => "bool".to_string(),
        DataType::Bytea => "bytes".to_string(),
        DataType::Uuid => "uuid".to_string(),
        DataType::Date => "date".to_string(),
        DataType::Time(_, tz) => match tz {
            TimezoneInfo::WithTimeZone | TimezoneInfo::Tz => "time_tz".to_string(),
            _ => "time".to_string(),
        },
        DataType::Timestamp(_, tz) => match tz {
            TimezoneInfo::WithTimeZone | TimezoneInfo::Tz => "datetime_tz".to_string(),
            _ => "datetime".to_string(),
        },
        DataType::Interval { .. } => "interval".to_string(),
        DataType::JSON => "json".to_string(),
        DataType::JSONB => "json".to_string(),
        DataType::Array(elem) => {
            let inner = match elem {
                ast::ArrayElemTypeDef::SquareBracket(inner_dt, _) => {
                    datatype_to_neutral(inner_dt, catalog)
                }
                ast::ArrayElemTypeDef::AngleBracket(inner_dt) => {
                    datatype_to_neutral(inner_dt, catalog)
                }
                ast::ArrayElemTypeDef::Parenthesis(inner_dt) => {
                    datatype_to_neutral(inner_dt, catalog)
                }
                ast::ArrayElemTypeDef::None => "unknown".to_string(),
            };
            format!("array<{}>", inner)
        }
        DataType::Custom(name, _) => {
            let raw = object_name_to_string(name).to_lowercase();
            match raw.as_str() {
                "timestamptz" => "datetime_tz".to_string(),
                "timetz" => "time_tz".to_string(),
                "serial" | "serial4" => "int32".to_string(),
                "bigserial" | "serial8" => "int64".to_string(),
                "smallserial" | "serial2" => "int16".to_string(),
                _ => sql_type_to_neutral(&raw, catalog),
            }
        }
        _ => {
            let s = dt.to_string().to_lowercase();
            sql_type_to_neutral(&s, catalog)
        }
    }
}

// ---------------------------------------------------------------------------
// Value matching helpers (ValueWithSpan wraps Value)
// ---------------------------------------------------------------------------

fn value_is_placeholder(vws: &ast::ValueWithSpan) -> Option<&str> {
    match &vws.value {
        Value::Placeholder(s) => Some(s.as_str()),
        _ => None,
    }
}

fn value_is_number(vws: &ast::ValueWithSpan) -> bool {
    matches!(&vws.value, Value::Number(_, _))
}

fn value_is_string(vws: &ast::ValueWithSpan) -> bool {
    matches!(
        &vws.value,
        Value::SingleQuotedString(_) | Value::DoubleQuotedString(_)
    )
}

fn value_is_boolean(vws: &ast::ValueWithSpan) -> bool {
    matches!(&vws.value, Value::Boolean(_))
}

fn value_is_null(vws: &ast::ValueWithSpan) -> bool {
    matches!(&vws.value, Value::Null)
}

// ---------------------------------------------------------------------------
// Analyzer implementation
// ---------------------------------------------------------------------------

impl<'a> Analyzer<'a> {
    fn analyze_statement(
        &mut self,
        stmt: &Statement,
    ) -> Result<(Vec<AnalyzedColumn>, Vec<ParamInfo>), ScytheError> {
        match stmt {
            Statement::Query(query) => {
                let cols = self.analyze_query(query)?;
                Ok((cols, self.params.clone()))
            }
            Statement::Insert(insert) => self.analyze_insert(insert),
            Statement::Update(update) => self.analyze_update(
                &update.table,
                &update.assignments,
                &update.from,
                &update.selection,
                &update.returning,
            ),
            Statement::Delete(delete) => self.analyze_delete(delete),
            _ => Ok((Vec::new(), Vec::new())),
        }
    }

    // -----------------------------------------------------------------------
    // SELECT / Query
    // -----------------------------------------------------------------------

    fn analyze_query(&mut self, query: &SqlQuery) -> Result<Vec<AnalyzedColumn>, ScytheError> {
        // Process CTEs first
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                let cte_name = cte.alias.name.value.to_lowercase();
                // Detect circular/recursive CTE referencing itself
                if with.recursive {
                    let is_union = matches!(cte.query.body.as_ref(), SetExpr::SetOperation { .. });
                    if !is_union {
                        // No UNION means direct self-reference (circular)
                        let body_sql = format!("{}", cte.query);
                        let body_lower = body_sql.to_lowercase();
                        if body_lower.contains(&format!("from {}", cte_name))
                            || body_lower.contains(&format!("join {}", cte_name))
                        {
                            return Err(ScytheError::invalid_recursion(format!(
                                "recursive CTE \"{}\" has no non-recursive base case",
                                cte_name
                            )));
                        }
                    } else {
                        // For recursive CTEs with UNION, analyze the base case first
                        // to get the CTE column types, then register before analyzing recursive part
                        if let SetExpr::SetOperation { left, .. } = cte.query.body.as_ref() {
                            let base_cols = self.analyze_set_expr(left)?;
                            let scope_cols: Vec<ScopeColumn> = base_cols
                                .iter()
                                .map(|c| ScopeColumn {
                                    name: c.name.clone(),
                                    neutral_type: c.neutral_type.clone(),
                                    base_nullable: c.nullable,
                                })
                                .collect();
                            self.ctes.push((cte_name.clone(), scope_cols));
                            // Now analyze the full CTE query (the recursive part will find "tree" in ctes)
                            let _ = self.analyze_query(&cte.query);
                            continue;
                        }
                    }
                }
                let cte_cols = self.analyze_query(&cte.query)?;
                let scope_cols: Vec<ScopeColumn> = cte_cols
                    .iter()
                    .map(|c| ScopeColumn {
                        name: c.name.clone(),
                        neutral_type: c.neutral_type.clone(),
                        base_nullable: c.nullable,
                    })
                    .collect();
                self.ctes.push((cte_name, scope_cols));
            }
        }

        // First pass to collect params from entire query
        let _ = self.analyze_set_expr(&query.body);

        // Handle LIMIT/OFFSET params
        if let Some(ref limit_clause) = query.limit_clause {
            match limit_clause {
                sqlparser::ast::LimitClause::LimitOffset { limit, offset, .. } => {
                    if let Some(limit) = limit {
                        self.collect_param_from_expr(limit, "limit_val", "int64");
                    }
                    if let Some(offset) = offset {
                        self.collect_param_from_expr(&offset.value, "offset_val", "int64");
                    }
                }
                sqlparser::ast::LimitClause::OffsetCommaLimit { offset, limit } => {
                    self.collect_param_from_expr(limit, "limit_val", "int64");
                    self.collect_param_from_expr(offset, "offset_val", "int64");
                }
            }
        }

        self.analyze_set_expr(&query.body)
    }

    fn analyze_set_expr(&mut self, set_expr: &SetExpr) -> Result<Vec<AnalyzedColumn>, ScytheError> {
        match set_expr {
            SetExpr::Select(select) => self.analyze_select(select),
            SetExpr::Query(query) => self.analyze_query(query),
            SetExpr::SetOperation { left, right, .. } => {
                // Use left side for column names, analyze right for params
                let left_cols = self.analyze_set_expr(left)?;
                let right_cols = self.analyze_set_expr(right)?;
                // Validate column count match
                if !left_cols.is_empty()
                    && !right_cols.is_empty()
                    && left_cols.len() != right_cols.len()
                {
                    return Err(ScytheError::column_count_mismatch(
                        left_cols.len(),
                        right_cols.len(),
                    ));
                }
                // Widen types across union
                let widened: Vec<AnalyzedColumn> = left_cols
                    .iter()
                    .enumerate()
                    .map(|(i, lc)| {
                        if i < right_cols.len() {
                            let widened_type =
                                widen_type(&lc.neutral_type, &right_cols[i].neutral_type);
                            AnalyzedColumn {
                                name: lc.name.clone(),
                                neutral_type: widened_type,
                                nullable: lc.nullable || right_cols[i].nullable,
                            }
                        } else {
                            lc.clone()
                        }
                    })
                    .collect();
                Ok(widened)
            }
            SetExpr::Values(values) => {
                if let Some(first_row) = values.rows.first() {
                    let cols: Vec<AnalyzedColumn> = first_row
                        .iter()
                        .enumerate()
                        .map(|(i, expr)| {
                            let ti = self.infer_expr_type(
                                expr,
                                &Scope {
                                    sources: Vec::new(),
                                },
                            );
                            AnalyzedColumn {
                                name: format!("column{}", i + 1),
                                neutral_type: ti.neutral_type,
                                nullable: ti.nullable,
                            }
                        })
                        .collect();
                    Ok(cols)
                } else {
                    Ok(Vec::new())
                }
            }
            _ => Ok(Vec::new()),
        }
    }

    fn analyze_select(&mut self, select: &ast::Select) -> Result<Vec<AnalyzedColumn>, ScytheError> {
        // 1. Build scope from FROM/JOIN
        let scope = self.build_scope_from_from(&select.from)?;

        // 2. Collect params from WHERE
        if let Some(ref selection) = select.selection {
            self.collect_params_from_where(selection, &scope);
        }

        // 3. Collect params from HAVING
        if let Some(ref having) = select.having {
            self.collect_params_from_where(having, &scope);
        }

        // 4. Resolve select items
        let mut columns = Vec::new();
        for item in &select.projection {
            match item {
                SelectItem::UnnamedExpr(expr) => {
                    let ti = self.infer_expr_type(expr, &scope);
                    let name = expr_to_name(expr);
                    columns.push(AnalyzedColumn {
                        name,
                        neutral_type: ti.neutral_type,
                        nullable: ti.nullable,
                    });
                }
                SelectItem::ExprWithAlias { expr, alias } => {
                    let ti = self.infer_expr_type(expr, &scope);
                    columns.push(AnalyzedColumn {
                        name: alias.value.to_lowercase(),
                        neutral_type: ti.neutral_type,
                        nullable: ti.nullable,
                    });
                }
                SelectItem::Wildcard(_) => {
                    for source in &scope.sources {
                        for col in &source.columns {
                            let nullable = col.base_nullable || source.nullable_from_join;
                            columns.push(AnalyzedColumn {
                                name: col.name.clone(),
                                neutral_type: col.neutral_type.clone(),
                                nullable,
                            });
                        }
                    }
                }
                SelectItem::QualifiedWildcard(kind, _) => {
                    let qualifier = match kind {
                        ast::SelectItemQualifiedWildcardKind::ObjectName(name) => {
                            object_name_to_string(name).to_lowercase()
                        }
                        ast::SelectItemQualifiedWildcardKind::Expr(expr) => expr_to_name(expr),
                    };
                    for source in &scope.sources {
                        if source.alias == qualifier || source.table_name == qualifier {
                            for col in &source.columns {
                                let nullable = col.base_nullable || source.nullable_from_join;
                                columns.push(AnalyzedColumn {
                                    name: col.name.clone(),
                                    neutral_type: col.neutral_type.clone(),
                                    nullable,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Validate: check for unknown columns, ambiguous columns, unknown functions
        for col in &columns {
            if let Some(name) = col.neutral_type.strip_prefix("__ambiguous__:") {
                return Err(ScytheError::ambiguous_column(name));
            }
            if let Some(name) = col.neutral_type.strip_prefix("__unknown_col__:") {
                return Err(ScytheError::unknown_column(name));
            }
            if let Some(name) = col.neutral_type.strip_prefix("__unknown_func__:") {
                return Err(ScytheError::unknown_function(name));
            }
        }

        // Check for duplicate aliases
        let mut seen_names: Vec<String> = Vec::new();
        for col in &columns {
            if seen_names.contains(&col.name) {
                return Err(ScytheError::duplicate_alias(&col.name));
            }
            seen_names.push(col.name.clone());
        }

        Ok(columns)
    }

    // -----------------------------------------------------------------------
    // Scope building
    // -----------------------------------------------------------------------

    fn build_scope_from_from(&self, from: &[TableWithJoins]) -> Result<Scope, ScytheError> {
        let mut scope = Scope {
            sources: Vec::new(),
        };

        for twj in from {
            self.add_table_factor_to_scope(&twj.relation, &mut scope, false)?;

            for join in &twj.joins {
                let nullable_from_join = match &join.join_operator {
                    JoinOperator::Left(_)
                    | JoinOperator::LeftOuter(_)
                    | JoinOperator::LeftSemi(_)
                    | JoinOperator::LeftAnti(_) => true,
                    JoinOperator::Right(_) | JoinOperator::RightOuter(_) => {
                        for source in &mut scope.sources {
                            source.nullable_from_join = true;
                        }
                        false
                    }
                    JoinOperator::FullOuter(_) => {
                        for source in &mut scope.sources {
                            source.nullable_from_join = true;
                        }
                        true
                    }
                    JoinOperator::CrossJoin(_)
                    | JoinOperator::Inner(_)
                    | JoinOperator::CrossApply
                    | JoinOperator::OuterApply => false,
                    _ => false,
                };

                self.add_table_factor_to_scope(&join.relation, &mut scope, nullable_from_join)?;
            }
        }

        Ok(scope)
    }

    fn add_table_factor_to_scope(
        &self,
        tf: &TableFactor,
        scope: &mut Scope,
        nullable_from_join: bool,
    ) -> Result<(), ScytheError> {
        match tf {
            TableFactor::Table { name, alias, .. } => {
                let table_name = object_name_to_string(name).to_lowercase();
                let alias_name = alias
                    .as_ref()
                    .map(|a| a.name.value.to_lowercase())
                    .unwrap_or_else(|| {
                        table_name
                            .rsplit_once('.')
                            .map(|(_, n)| n.to_string())
                            .unwrap_or_else(|| table_name.clone())
                    });

                // Look up in CTEs first
                let scope_cols = if let Some((_, cte_cols)) =
                    self.ctes.iter().find(|(n, _)| *n == table_name)
                {
                    cte_cols.clone()
                } else if let Some(table) = self.catalog.get_table(&table_name) {
                    table
                        .columns
                        .iter()
                        .map(|c| ScopeColumn {
                            name: c.name.clone(),
                            neutral_type: sql_type_to_neutral(&c.sql_type, self.catalog),
                            base_nullable: c.nullable,
                        })
                        .collect()
                } else {
                    // Check if this is a known set-returning function (table functions)
                    let known_functions = [
                        "generate_series",
                        "unnest",
                        "jsonb_array_elements",
                        "json_array_elements",
                        "jsonb_each",
                        "json_each",
                        "jsonb_each_text",
                        "json_each_text",
                        "jsonb_object_keys",
                        "json_object_keys",
                        "jsonb_populate_record",
                        "json_populate_record",
                        "jsonb_populate_recordset",
                        "json_populate_recordset",
                        "regexp_matches",
                        "string_to_table",
                    ];
                    if known_functions.contains(&table_name.as_str()) {
                        // Return function-specific columns
                        match table_name.as_str() {
                            "jsonb_array_elements" | "json_array_elements" => vec![ScopeColumn {
                                name: "value".to_string(),
                                neutral_type: "json".to_string(),
                                base_nullable: true,
                            }],
                            "jsonb_each" | "json_each" => vec![
                                ScopeColumn {
                                    name: "key".to_string(),
                                    neutral_type: "string".to_string(),
                                    base_nullable: false,
                                },
                                ScopeColumn {
                                    name: "value".to_string(),
                                    neutral_type: "json".to_string(),
                                    base_nullable: true,
                                },
                            ],
                            "jsonb_each_text" | "json_each_text" => vec![
                                ScopeColumn {
                                    name: "key".to_string(),
                                    neutral_type: "string".to_string(),
                                    base_nullable: false,
                                },
                                ScopeColumn {
                                    name: "value".to_string(),
                                    neutral_type: "string".to_string(),
                                    base_nullable: true,
                                },
                            ],
                            "jsonb_object_keys" | "json_object_keys" => vec![ScopeColumn {
                                name: "jsonb_object_keys".to_string(),
                                neutral_type: "string".to_string(),
                                base_nullable: false,
                            }],
                            _ => Vec::new(),
                        }
                    } else {
                        return Err(ScytheError::new(
                            crate::errors::ErrorCode::UnknownTable,
                            format!("relation \"{}\" does not exist", table_name),
                        ));
                    }
                };

                scope.sources.push(ScopeSource {
                    alias: alias_name,
                    table_name,
                    columns: scope_cols,
                    nullable_from_join,
                });
            }
            TableFactor::Derived {
                subquery, alias, ..
            } => {
                // Analyze subquery to get columns
                let mut sub_analyzer = Analyzer {
                    catalog: self.catalog,
                    params: Vec::new(),
                    ctes: self.ctes.clone(),
                    type_errors: Vec::new(),
                };
                let sub_cols = sub_analyzer.analyze_query(subquery).unwrap_or_default();

                let alias_name = alias
                    .as_ref()
                    .map(|a| a.name.value.to_lowercase())
                    .unwrap_or_else(|| "subquery".to_string());

                let scope_cols: Vec<ScopeColumn> = sub_cols
                    .iter()
                    .map(|c| ScopeColumn {
                        name: c.name.clone(),
                        neutral_type: c.neutral_type.clone(),
                        base_nullable: c.nullable,
                    })
                    .collect();

                scope.sources.push(ScopeSource {
                    alias: alias_name.clone(),
                    table_name: alias_name,
                    columns: scope_cols,
                    nullable_from_join,
                });
            }
            TableFactor::NestedJoin {
                table_with_joins,
                alias: _,
            } => {
                self.add_table_factor_to_scope(
                    &table_with_joins.relation,
                    scope,
                    nullable_from_join,
                )?;
                for join in &table_with_joins.joins {
                    let join_nullable = match &join.join_operator {
                        JoinOperator::Left(_) | JoinOperator::LeftOuter(_) => true,
                        JoinOperator::Right(_) | JoinOperator::RightOuter(_) => {
                            for source in scope.sources.iter_mut() {
                                source.nullable_from_join = true;
                            }
                            false
                        }
                        JoinOperator::FullOuter(_) => {
                            for source in scope.sources.iter_mut() {
                                source.nullable_from_join = true;
                            }
                            true
                        }
                        _ => nullable_from_join,
                    };
                    self.add_table_factor_to_scope(&join.relation, scope, join_nullable)?;
                }
            }
            TableFactor::TableFunction { alias, .. } | TableFactor::UNNEST { alias, .. } => {
                let alias_name = alias
                    .as_ref()
                    .map(|a| a.name.value.to_lowercase())
                    .unwrap_or_else(|| "func".to_string());
                scope.sources.push(ScopeSource {
                    alias: alias_name.clone(),
                    table_name: alias_name,
                    columns: Vec::new(),
                    nullable_from_join,
                });
            }
            TableFactor::Function { alias, name, .. } => {
                let alias_name = alias
                    .as_ref()
                    .map(|a| a.name.value.to_lowercase())
                    .unwrap_or_else(|| object_name_to_string(name).to_lowercase());

                let func_name = object_name_to_string(name).to_lowercase();
                let cols = match func_name.as_str() {
                    "generate_series" => vec![ScopeColumn {
                        name: "generate_series".to_string(),
                        neutral_type: "int32".to_string(),
                        base_nullable: false,
                    }],
                    "unnest" => vec![ScopeColumn {
                        name: "unnest".to_string(),
                        neutral_type: "unknown".to_string(),
                        base_nullable: true,
                    }],
                    "jsonb_array_elements" | "json_array_elements" => vec![ScopeColumn {
                        name: "value".to_string(),
                        neutral_type: "json".to_string(),
                        base_nullable: true,
                    }],
                    "jsonb_each" | "json_each" => vec![
                        ScopeColumn {
                            name: "key".to_string(),
                            neutral_type: "string".to_string(),
                            base_nullable: false,
                        },
                        ScopeColumn {
                            name: "value".to_string(),
                            neutral_type: "json".to_string(),
                            base_nullable: true,
                        },
                    ],
                    "jsonb_each_text" | "json_each_text" => vec![
                        ScopeColumn {
                            name: "key".to_string(),
                            neutral_type: "string".to_string(),
                            base_nullable: false,
                        },
                        ScopeColumn {
                            name: "value".to_string(),
                            neutral_type: "string".to_string(),
                            base_nullable: true,
                        },
                    ],
                    "jsonb_object_keys" | "json_object_keys" => vec![ScopeColumn {
                        name: "jsonb_object_keys".to_string(),
                        neutral_type: "string".to_string(),
                        base_nullable: false,
                    }],
                    _ => Vec::new(),
                };

                // If alias has column definitions, use those
                let cols = if let Some(a) = alias {
                    if !a.columns.is_empty() {
                        a.columns
                            .iter()
                            .map(|c| ScopeColumn {
                                name: c.name.value.to_lowercase(),
                                neutral_type: "unknown".to_string(),
                                base_nullable: true,
                            })
                            .collect()
                    } else {
                        cols
                    }
                } else {
                    cols
                };

                scope.sources.push(ScopeSource {
                    alias: alias_name.clone(),
                    table_name: alias_name,
                    columns: cols,
                    nullable_from_join,
                });
            }
            _ => {}
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Expression type inference
    // -----------------------------------------------------------------------

    fn infer_expr_type(&mut self, expr: &Expr, scope: &Scope) -> TypeInfo {
        match expr {
            Expr::Identifier(ident) => {
                let col_name = if ident.quote_style.is_some() {
                    ident.value.clone()
                } else {
                    ident.value.to_lowercase()
                };
                self.resolve_column_in_scope(&col_name, None, scope)
            }

            Expr::CompoundIdentifier(parts) => {
                if parts.len() == 2 {
                    let qualifier = parts[0].value.to_lowercase();
                    let col_name = parts[1].value.to_lowercase();
                    self.resolve_column_in_scope(&col_name, Some(&qualifier), scope)
                } else if parts.len() >= 3 {
                    let qualifier = parts[parts.len() - 2].value.to_lowercase();
                    let col_name = parts[parts.len() - 1].value.to_lowercase();
                    self.resolve_column_in_scope(&col_name, Some(&qualifier), scope)
                } else {
                    TypeInfo::unknown()
                }
            }

            Expr::Value(vws) => {
                if value_is_number(vws) {
                    TypeInfo::new("int64", false)
                } else if value_is_string(vws) {
                    TypeInfo::new("string", false)
                } else if value_is_boolean(vws) {
                    TypeInfo::new("bool", false)
                } else if value_is_null(vws) {
                    TypeInfo::new("unknown", true)
                } else if let Some(p) = value_is_placeholder(vws) {
                    if let Some(pos) = parse_placeholder(p) {
                        self.register_param(pos, None, None, false);
                    }
                    TypeInfo::unknown()
                } else {
                    TypeInfo::new("string", false)
                }
            }

            Expr::Cast {
                expr: inner,
                data_type,
                ..
            } => {
                let inner_ti = self.infer_expr_type(inner, scope);
                let neutral = datatype_to_neutral(data_type, self.catalog);
                self.collect_param_type_from_cast(inner, &neutral);
                TypeInfo::new(neutral, inner_ti.nullable)
            }

            Expr::Function(func) => self.infer_function_type(func, scope),

            Expr::BinaryOp { left, op, right } => {
                let left_ti = self.infer_expr_type(left, scope);
                let right_ti = self.infer_expr_type(right, scope);

                match op {
                    BinaryOperator::StringConcat => {
                        TypeInfo::new("string", left_ti.nullable || right_ti.nullable)
                    }
                    BinaryOperator::Plus
                    | BinaryOperator::Minus
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide
                    | BinaryOperator::Modulo => {
                        let result_type = if left_ti.neutral_type == "unknown" {
                            right_ti.neutral_type.clone()
                        } else {
                            left_ti.neutral_type.clone()
                        };
                        TypeInfo::new(result_type, left_ti.nullable || right_ti.nullable)
                    }
                    BinaryOperator::Eq
                    | BinaryOperator::NotEq
                    | BinaryOperator::Lt
                    | BinaryOperator::LtEq
                    | BinaryOperator::Gt
                    | BinaryOperator::GtEq
                    | BinaryOperator::And
                    | BinaryOperator::Or => TypeInfo::new("bool", false),
                    BinaryOperator::Arrow => TypeInfo::new("json", true),
                    BinaryOperator::LongArrow => TypeInfo::new("string", true),
                    BinaryOperator::HashArrow => TypeInfo::new("json", true),
                    BinaryOperator::HashLongArrow => TypeInfo::new("string", true),
                    _ => TypeInfo::new(left_ti.neutral_type, left_ti.nullable || right_ti.nullable),
                }
            }

            Expr::UnaryOp { op, expr: inner } => {
                let ti = self.infer_expr_type(inner, scope);
                match op {
                    UnaryOperator::Not => TypeInfo::new("bool", ti.nullable),
                    UnaryOperator::Minus | UnaryOperator::Plus => ti,
                    _ => ti,
                }
            }

            Expr::Nested(inner) => self.infer_expr_type(inner, scope),

            Expr::IsNull(_) | Expr::IsNotNull(_) => TypeInfo::new("bool", false),

            Expr::IsTrue(_)
            | Expr::IsFalse(_)
            | Expr::IsNotTrue(_)
            | Expr::IsNotFalse(_)
            | Expr::IsUnknown(_)
            | Expr::IsNotUnknown(_) => TypeInfo::new("bool", false),

            Expr::InList {
                expr: col_expr,
                list,
                ..
            } => {
                let col_ti = self.infer_expr_type(col_expr, scope);
                for item in list {
                    if let Expr::Value(vws) = item
                        && let Some(p) = value_is_placeholder(vws)
                        && let Some(pos) = parse_placeholder(p)
                    {
                        let col_name = expr_to_name(col_expr);
                        self.register_param(
                            pos,
                            Some(col_name),
                            Some(col_ti.neutral_type.clone()),
                            false,
                        );
                    }
                }
                TypeInfo::new("bool", false)
            }

            Expr::InSubquery { .. } => TypeInfo::new("bool", false),

            Expr::Between {
                expr: col_expr,
                low,
                high,
                ..
            } => {
                let col_ti = self.infer_expr_type(col_expr, scope);
                let _col_name = expr_to_name(col_expr);
                self.collect_param_from_expr_with_type(low, &col_ti.neutral_type, "start");
                self.collect_param_from_expr_with_type(high, &col_ti.neutral_type, "end");
                TypeInfo::new("bool", false)
            }

            Expr::Like {
                expr: col_expr,
                pattern,
                ..
            }
            | Expr::ILike {
                expr: col_expr,
                pattern,
                ..
            } => {
                let _col_ti = self.infer_expr_type(col_expr, scope);
                self.collect_param_from_expr_with_type(pattern, "string", &expr_to_name(col_expr));
                TypeInfo::new("bool", false)
            }

            Expr::Case {
                operand: _,
                conditions,
                else_result,
                ..
            } => {
                let mut result_type = "unknown".to_string();
                let mut any_nullable = false;

                for case_when in conditions {
                    // Infer condition type (may contain params)
                    let _ = self.infer_expr_type(&case_when.condition, scope);
                    // If condition is a placeholder, it's a bool flag
                    if let Expr::Value(vws) = &case_when.condition
                        && let Some(p) = value_is_placeholder(vws)
                        && let Some(pos) = parse_placeholder(p)
                    {
                        self.register_param(
                            pos,
                            Some("flag".to_string()),
                            Some("bool".to_string()),
                            false,
                        );
                    }

                    // Infer result type
                    let ti = self.infer_expr_type(&case_when.result, scope);
                    if result_type == "unknown" && ti.neutral_type != "unknown" {
                        result_type = ti.neutral_type.clone();
                    }
                    // Check if condition is IS NOT NULL on the same expr as result
                    let guarded = is_not_null_guard(&case_when.condition, &case_when.result);
                    if ti.nullable && !guarded {
                        any_nullable = true;
                    }
                }

                if let Some(else_expr) = else_result {
                    let ti = self.infer_expr_type(else_expr, scope);
                    if result_type == "unknown" && ti.neutral_type != "unknown" {
                        result_type = ti.neutral_type.clone();
                    }
                    if ti.nullable {
                        any_nullable = true;
                    }
                } else {
                    any_nullable = true;
                }

                TypeInfo::new(result_type, any_nullable)
            }

            Expr::Subquery(query) => {
                if let Ok(cols) = self.analyze_query(query)
                    && let Some(first) = cols.first()
                {
                    // Scalar subquery: if the subquery returns a non-nullable aggregate
                    // (like COUNT), the result is non-nullable
                    return TypeInfo::new(first.neutral_type.clone(), first.nullable);
                }
                TypeInfo::unknown()
            }

            Expr::Exists { .. } => TypeInfo::new("bool", false),

            Expr::AnyOp { left, right, .. } => {
                let left_ti = self.infer_expr_type(left, scope);
                if let Expr::Value(vws) = right.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = parse_placeholder(p)
                {
                    let array_type = format!("array<{}>", left_ti.neutral_type);
                    let name = pluralize(&expr_to_name(left));
                    self.register_param(pos, Some(name), Some(array_type), false);
                }
                self.collect_param_from_any(right, &left_ti, &expr_to_name(left));
                TypeInfo::new("bool", false)
            }

            Expr::AllOp { left, right, .. } => {
                let left_ti = self.infer_expr_type(left, scope);
                if let Expr::Value(vws) = right.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = parse_placeholder(p)
                {
                    let array_type = format!("array<{}>", left_ti.neutral_type);
                    let name = pluralize(&expr_to_name(left));
                    self.register_param(pos, Some(name), Some(array_type), false);
                }
                TypeInfo::new("bool", false)
            }

            Expr::Array(arr) => {
                if let Some(first) = arr.elem.first() {
                    let ti = self.infer_expr_type(first, scope);
                    TypeInfo::new(format!("array<{}>", ti.neutral_type), false)
                } else {
                    TypeInfo::new("array<unknown>", false)
                }
            }

            Expr::Tuple(exprs) => {
                if let Some(first) = exprs.first() {
                    self.infer_expr_type(first, scope)
                } else {
                    TypeInfo::unknown()
                }
            }

            Expr::Extract { expr, .. } => {
                let ti = self.infer_expr_type(expr, scope);
                TypeInfo::new("float64", ti.nullable)
            }

            Expr::Substring { expr, .. } => {
                let ti = self.infer_expr_type(expr, scope);
                TypeInfo::new("string", ti.nullable)
            }

            Expr::Trim { expr, .. } => {
                let ti = self.infer_expr_type(expr, scope);
                TypeInfo::new("string", ti.nullable)
            }

            Expr::Position { .. } => TypeInfo::new("int32", false),

            Expr::AtTimeZone { timestamp, .. } => {
                let ti = self.infer_expr_type(timestamp, scope);
                if ti.neutral_type == "datetime_tz" {
                    TypeInfo::new("datetime", ti.nullable)
                } else {
                    TypeInfo::new("datetime_tz", ti.nullable)
                }
            }

            Expr::TypedString(ts) => {
                let neutral = datatype_to_neutral(&ts.data_type, self.catalog);
                TypeInfo::new(neutral, false)
            }

            Expr::Interval { .. } => TypeInfo::new("interval", false),

            Expr::CompoundFieldAccess { root, access_chain } => {
                // e.g. (home_address).street — resolve the composite type and field
                let root_ti = self.infer_expr_type(root, scope);
                // Check if root is a composite type
                if let Some(comp_name) = root_ti.neutral_type.strip_prefix("composite::")
                    && let Some(comp) = self.catalog.get_composite(comp_name)
                {
                    // Get the last field access
                    if let Some(last) = access_chain.last()
                        && let ast::AccessExpr::Dot(Expr::Identifier(ident)) = last
                    {
                        let field_name = ident.value.to_lowercase();
                        if let Some(field) = comp.fields.iter().find(|f| f.name == field_name) {
                            let neutral = sql_type_to_neutral(&field.sql_type, self.catalog);
                            // Composite fields are nullable even if composite is NOT NULL
                            return TypeInfo::new(neutral, true);
                        }
                    }
                }
                TypeInfo::unknown()
            }

            Expr::Ceil { expr: inner, .. } | Expr::Floor { expr: inner, .. } => {
                let ti = self.infer_expr_type(inner, scope);
                TypeInfo::new(ti.neutral_type, ti.nullable)
            }

            _ => TypeInfo::unknown(),
        }
    }

    fn resolve_column_in_scope(
        &self,
        col_name: &str,
        qualifier: Option<&str>,
        scope: &Scope,
    ) -> TypeInfo {
        if let Some(qual) = qualifier {
            for source in &scope.sources {
                if (source.alias == qual || source.table_name == qual)
                    && let Some(col) = source.columns.iter().find(|c| c.name == col_name)
                {
                    let nullable = col.base_nullable || source.nullable_from_join;
                    return TypeInfo::new(col.neutral_type.clone(), nullable);
                }
            }
        } else {
            let mut found: Option<TypeInfo> = None;
            for source in &scope.sources {
                if let Some(col) = source.columns.iter().find(|c| c.name == col_name) {
                    let nullable = col.base_nullable || source.nullable_from_join;
                    let ti = TypeInfo::new(col.neutral_type.clone(), nullable);
                    if found.is_some() {
                        // ambiguous - mark it
                        return TypeInfo {
                            neutral_type: format!("__ambiguous__:{}", col_name),
                            nullable: false,
                        };
                    }
                    found = Some(ti);
                }
            }
            if let Some(ti) = found {
                return ti;
            }
        }

        // Check if scope has any sources with columns at all (for error detection)
        let has_sources = scope.sources.iter().any(|s| !s.columns.is_empty());
        if has_sources {
            return TypeInfo {
                neutral_type: format!("__unknown_col__:{}", col_name),
                nullable: true,
            };
        }

        TypeInfo::unknown()
    }

    // -----------------------------------------------------------------------
    // Function type inference
    // -----------------------------------------------------------------------

    fn infer_function_type(&mut self, func: &ast::Function, scope: &Scope) -> TypeInfo {
        let func_name = object_name_to_string(&func.name).to_lowercase();
        let is_window = func.over.is_some();

        let first_arg_ti = self.get_first_arg_type(func, scope);
        let first_arg_nullable = first_arg_ti.as_ref().map(|ti| ti.nullable).unwrap_or(true);

        match func_name.as_str() {
            "count" => TypeInfo::new("int64", false),
            "sum" => {
                let base_type = first_arg_ti
                    .as_ref()
                    .map(|ti| {
                        if is_integer_type(&ti.neutral_type) {
                            "int64".to_string()
                        } else {
                            "decimal".to_string()
                        }
                    })
                    .unwrap_or_else(|| "int64".to_string());
                if is_window {
                    TypeInfo::new(base_type, false)
                } else {
                    TypeInfo::new(base_type, true)
                }
            }
            "avg" => {
                if is_window {
                    TypeInfo::new("decimal", false)
                } else {
                    TypeInfo::new("decimal", true)
                }
            }
            "min" | "max" => {
                let base_type = first_arg_ti
                    .as_ref()
                    .map(|ti| ti.neutral_type.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                if is_window {
                    TypeInfo::new(base_type, first_arg_nullable)
                } else {
                    TypeInfo::new(base_type, true)
                }
            }
            "string_agg" | "array_agg" => {
                let base_type = if func_name == "string_agg" {
                    "string".to_string()
                } else {
                    let inner = first_arg_ti
                        .as_ref()
                        .map(|ti| ti.neutral_type.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    format!("array<{}>", inner)
                };
                TypeInfo::new(base_type, true)
            }
            "bool_and" | "bool_or" | "every" => TypeInfo::new("bool", true),
            "json_agg" | "jsonb_agg" | "json_object_agg" | "jsonb_object_agg" => {
                TypeInfo::new("json", true)
            }

            "coalesce" => {
                let args = self.get_function_args(func);
                let mut result_type = "unknown".to_string();
                let mut any_non_nullable = false;
                // Get a name from the first non-placeholder argument
                let mut coalesce_name: Option<String> = None;

                for arg in &args {
                    let ti = self.infer_expr_type(arg, scope);
                    if result_type == "unknown" && ti.neutral_type != "unknown" {
                        result_type = ti.neutral_type.clone();
                    }
                    if !ti.nullable || is_literal(arg) {
                        any_non_nullable = true;
                    }
                    if coalesce_name.is_none()
                        && !matches!(arg, Expr::Value(vws) if value_is_placeholder(vws).is_some())
                    {
                        let n = expr_to_name(arg);
                        if n != "unknown" {
                            coalesce_name = Some(n);
                        }
                    }
                }

                for arg in &args {
                    if let Expr::Value(vws) = arg
                        && let Some(p) = value_is_placeholder(vws)
                        && let Some(pos) = parse_placeholder(p)
                    {
                        let param_type = if result_type != "unknown" {
                            Some(result_type.clone())
                        } else {
                            None
                        };
                        self.register_param(pos, coalesce_name.clone(), param_type, true);
                    }
                }

                TypeInfo::new(result_type, !any_non_nullable)
            }

            "nullif" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo::new(ti.neutral_type, true)
            }

            // String functions
            "upper" | "lower" | "initcap" | "reverse" | "ltrim" | "rtrim" | "btrim" | "lpad"
            | "rpad" | "repeat" | "replace" | "translate" | "left" | "right" | "md5" | "encode"
            | "decode" | "chr" | "to_hex" | "quote_ident" | "quote_literal" | "format"
            | "regexp_replace" => TypeInfo::new("string", first_arg_nullable),
            "concat" | "concat_ws" => TypeInfo::new("string", false),
            "substring" | "substr" => TypeInfo::new("string", first_arg_nullable),
            "length" | "char_length" | "character_length" | "octet_length" | "bit_length"
            | "strpos" => TypeInfo::new("int32", first_arg_nullable),

            // Math functions
            "abs" | "sign" => first_arg_ti.unwrap_or_else(TypeInfo::unknown),
            "ceil" | "ceiling" | "floor" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo::new(ti.neutral_type, ti.nullable)
            }
            "round" | "trunc" => TypeInfo::new("decimal", first_arg_nullable),
            "power" | "sqrt" | "cbrt" | "log" | "ln" | "exp" | "pi" | "sin" | "cos" | "tan"
            | "asin" | "acos" | "atan" | "atan2" | "degrees" | "radians" | "random" => {
                TypeInfo::new("float64", false)
            }
            "mod" => first_arg_ti.unwrap_or_else(|| TypeInfo::new("int32", false)),
            "div" => TypeInfo::new("int64", first_arg_nullable),
            "greatest" | "least" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo::new(ti.neutral_type, ti.nullable)
            }

            // Date/time functions
            "now"
            | "current_timestamp"
            | "statement_timestamp"
            | "transaction_timestamp"
            | "clock_timestamp" => TypeInfo::new("datetime_tz", false),
            "current_date" | "localdate" => TypeInfo::new("date", false),
            "current_time" | "localtime" => TypeInfo::new("time_tz", false),
            "date_trunc" => {
                let args = self.get_function_args(func);
                if args.len() >= 2 {
                    let ti = self.infer_expr_type(&args[1], scope);
                    TypeInfo::new(ti.neutral_type, ti.nullable)
                } else {
                    TypeInfo::new("datetime_tz", first_arg_nullable)
                }
            }
            "date_part" | "extract" => TypeInfo::new("float64", first_arg_nullable),
            "age" => TypeInfo::new("interval", false),
            "make_date" => TypeInfo::new("date", false),
            "make_time" => TypeInfo::new("time", false),
            "make_timestamp" => TypeInfo::new("datetime", false),
            "make_timestamptz" => TypeInfo::new("datetime_tz", false),
            "make_interval" => TypeInfo::new("interval", false),
            "to_timestamp" => TypeInfo::new("datetime_tz", false),
            "to_date" => TypeInfo::new("date", false),
            "to_char" => TypeInfo::new("string", first_arg_nullable),

            // Window functions
            "row_number" | "rank" | "dense_rank" | "cume_dist" | "ntile" | "percent_rank" => {
                TypeInfo::new("int64", false)
            }
            "lag" | "lead" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo::new(ti.neutral_type, true)
            }
            "first_value" | "last_value" | "nth_value" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo::new(ti.neutral_type, true)
            }

            // JSON functions
            "json_build_object" | "jsonb_build_object" | "json_build_array"
            | "jsonb_build_array" | "to_json" | "to_jsonb" | "json_strip_nulls"
            | "jsonb_strip_nulls" => TypeInfo::new("json", false),
            "json_typeof" | "jsonb_typeof" => TypeInfo::new("string", true),
            "json_extract_path_text" | "jsonb_extract_path_text" => TypeInfo::new("string", true),
            "json_extract_path" | "jsonb_extract_path" => TypeInfo::new("json", true),
            "json_array_length" | "jsonb_array_length" => TypeInfo::new("int32", true),
            "json_each" | "jsonb_each" | "json_each_text" | "jsonb_each_text" => {
                TypeInfo::new("string", true)
            }
            "json_object_keys" | "jsonb_object_keys" => TypeInfo::new("string", false),
            "json_populate_record"
            | "jsonb_populate_record"
            | "json_populate_recordset"
            | "jsonb_populate_recordset" => TypeInfo::new("unknown", true),

            // Array functions
            "array_length" | "array_ndims" | "array_lower" | "array_upper" | "cardinality" => {
                TypeInfo::new("int32", true)
            }
            "array_cat" | "array_append" | "array_prepend" | "array_remove" | "array_replace"
            | "array_positions" => first_arg_ti.unwrap_or_else(TypeInfo::unknown),
            "array_position" => TypeInfo::new("int32", true),
            "array_to_string" => TypeInfo::new("string", true),
            "unnest" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                let inner =
                    if ti.neutral_type.starts_with("array<") && ti.neutral_type.ends_with('>') {
                        ti.neutral_type[6..ti.neutral_type.len() - 1].to_string()
                    } else {
                        "unknown".to_string()
                    };
                TypeInfo::new(inner, true)
            }

            // Misc
            "gen_random_uuid" | "uuid_generate_v4" => TypeInfo::new("uuid", false),
            "nextval" | "currval" | "lastval" | "setval" => TypeInfo::new("int64", false),
            "pg_typeof" => TypeInfo::new("string", false),

            _ => {
                // Mark as unknown function for error detection
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo {
                    neutral_type: format!("__unknown_func__:{}", func_name),
                    nullable: ti.nullable,
                }
            }
        }
    }

    fn get_first_arg_type(&mut self, func: &ast::Function, scope: &Scope) -> Option<TypeInfo> {
        let args = self.get_function_args(func);
        args.first().map(|arg| self.infer_expr_type(arg, scope))
    }

    fn get_function_args(&self, func: &ast::Function) -> Vec<Expr> {
        match &func.args {
            ast::FunctionArguments::None => Vec::new(),
            ast::FunctionArguments::Subquery(_) => Vec::new(),
            ast::FunctionArguments::List(arg_list) => arg_list
                .args
                .iter()
                .filter_map(|arg| match arg {
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => Some(e.clone()),
                    FunctionArg::Named {
                        arg: FunctionArgExpr::Expr(e),
                        ..
                    } => Some(e.clone()),
                    _ => None,
                })
                .collect(),
        }
    }

    // -----------------------------------------------------------------------
    // Parameter collection
    // -----------------------------------------------------------------------

    fn register_param(
        &mut self,
        position: i64,
        name: Option<String>,
        neutral_type: Option<String>,
        nullable: bool,
    ) {
        if let Some(existing) = self.params.iter_mut().find(|p| p.position == position) {
            if existing.name.is_none() && name.is_some() {
                existing.name = name;
            }
            if existing.neutral_type.is_none() && neutral_type.is_some() {
                existing.neutral_type = neutral_type;
            }
        } else {
            self.params.push(ParamInfo {
                position,
                name,
                neutral_type,
                nullable,
            });
        }
    }

    fn collect_params_from_where(&mut self, expr: &Expr, scope: &Scope) {
        match expr {
            Expr::BinaryOp { left, op, right } => match op {
                BinaryOperator::Eq
                | BinaryOperator::NotEq
                | BinaryOperator::Lt
                | BinaryOperator::LtEq
                | BinaryOperator::Gt
                | BinaryOperator::GtEq => {
                    self.try_bind_param_from_comparison(left, right, scope, Some(op));
                    self.try_bind_param_from_comparison(right, left, scope, Some(op));
                    // Type mismatch detection: store for validation
                    let left_ti = self.infer_expr_type(left, scope);
                    let right_ti = self.infer_expr_type(right, scope);
                    if left_ti.neutral_type != "unknown"
                        && right_ti.neutral_type != "unknown"
                        && !left_ti.neutral_type.starts_with("__")
                        && !right_ti.neutral_type.starts_with("__")
                        && left_ti.neutral_type != right_ti.neutral_type
                        && !is_comparable_types(&left_ti.neutral_type, &right_ti.neutral_type)
                    {
                        let left_sql = neutral_to_sql_label(&left_ti.neutral_type);
                        let right_sql = neutral_to_sql_label(&right_ti.neutral_type);
                        let op_str = format!("{}", op);
                        self.type_errors.push(format!(
                            "operator does not exist: {} {} {}",
                            left_sql, op_str, right_sql
                        ));
                    }
                }
                BinaryOperator::And | BinaryOperator::Or => {
                    self.collect_params_from_where(left, scope);
                    self.collect_params_from_where(right, scope);
                }
                _ => {
                    self.collect_params_from_where(left, scope);
                    self.collect_params_from_where(right, scope);
                }
            },
            Expr::Between {
                expr: col_expr,
                low,
                high,
                ..
            } => {
                let col_ti = self.infer_expr_type(col_expr, scope);
                self.collect_param_from_expr_with_type(low, &col_ti.neutral_type, "start");
                self.collect_param_from_expr_with_type(high, &col_ti.neutral_type, "end");
            }
            Expr::Like {
                expr: col_expr,
                pattern,
                ..
            }
            | Expr::ILike {
                expr: col_expr,
                pattern,
                ..
            } => {
                if let Expr::Value(vws) = pattern.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = parse_placeholder(p)
                {
                    let name = expr_to_name(col_expr);
                    self.register_param(pos, Some(name), Some("string".to_string()), false);
                }
            }
            Expr::InList {
                expr: col_expr,
                list,
                ..
            } => {
                let col_ti = self.infer_expr_type(col_expr, scope);
                let col_name = expr_to_name(col_expr);
                for item in list {
                    if let Expr::Value(vws) = item
                        && let Some(p) = value_is_placeholder(vws)
                        && let Some(pos) = parse_placeholder(p)
                    {
                        self.register_param(
                            pos,
                            Some(col_name.clone()),
                            Some(col_ti.neutral_type.clone()),
                            false,
                        );
                    }
                }
            }
            Expr::IsNull(_) | Expr::IsNotNull(_) => {}
            Expr::Nested(inner) => self.collect_params_from_where(inner, scope),
            Expr::UnaryOp { expr: inner, .. } => self.collect_params_from_where(inner, scope),
            Expr::AnyOp { left, right, .. } => {
                let left_ti = self.infer_expr_type(left, scope);
                if let Expr::Value(vws) = right.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = parse_placeholder(p)
                {
                    let array_type = format!("array<{}>", left_ti.neutral_type);
                    let name = pluralize(&expr_to_name(left));
                    self.register_param(pos, Some(name), Some(array_type), false);
                }
                self.collect_param_from_any(right, &left_ti, &expr_to_name(left));
            }
            Expr::InSubquery { subquery, .. } => {
                let _ = self.analyze_query(subquery);
            }
            Expr::Exists { subquery, .. } => {
                let _ = self.analyze_query(subquery);
            }
            Expr::Subquery(subquery) => {
                let _ = self.analyze_query(subquery);
            }
            Expr::Case {
                conditions,
                else_result,
                ..
            } => {
                for case_when in conditions {
                    self.collect_params_from_where(&case_when.condition, scope);
                    let _ = self.infer_expr_type(&case_when.result, scope);
                }
                if let Some(else_expr) = else_result {
                    let _ = self.infer_expr_type(else_expr, scope);
                }
            }
            Expr::Function(func) => {
                let _ = self.infer_function_type(func, scope);
            }
            Expr::Cast {
                expr: inner,
                data_type,
                ..
            } => {
                let neutral = datatype_to_neutral(data_type, self.catalog);
                self.collect_param_type_from_cast(inner, &neutral);
                self.collect_params_from_where(inner, scope);
            }
            _ => {
                let _ = self.infer_expr_type(expr, scope);
            }
        }
    }

    fn try_bind_param_from_comparison(
        &mut self,
        param_side: &Expr,
        col_side: &Expr,
        scope: &Scope,
        op: Option<&BinaryOperator>,
    ) {
        match param_side {
            Expr::Value(vws) => {
                if let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = parse_placeholder(p)
                {
                    let col_ti = self.infer_expr_type(col_side, scope);
                    let col_name = expr_to_name(col_side);
                    // Add prefix for aggregate comparisons in HAVING
                    let param_name =
                        derive_param_name_from_comparison(&col_name, col_side, param_side, op);
                    self.register_param(pos, Some(param_name), Some(col_ti.neutral_type), false);
                }
            }
            Expr::Cast {
                expr: inner,
                data_type,
                ..
            } => {
                if let Expr::Value(vws) = inner.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = parse_placeholder(p)
                {
                    let neutral = datatype_to_neutral(data_type, self.catalog);
                    let col_name = expr_to_name(col_side);
                    let param_name =
                        derive_param_name_from_comparison(&col_name, col_side, param_side, op);
                    self.register_param(pos, Some(param_name), Some(neutral), false);
                }
            }
            _ => {}
        }
    }

    fn collect_param_from_expr(&mut self, expr: &Expr, name: &str, type_str: &str) {
        if let Expr::Value(vws) = expr {
            if let Some(p) = value_is_placeholder(vws)
                && let Some(pos) = parse_placeholder(p)
            {
                self.register_param(
                    pos,
                    Some(name.to_string()),
                    Some(type_str.to_string()),
                    false,
                );
            }
        } else if let Expr::Cast {
            expr: inner,
            data_type,
            ..
        } = expr
            && let Expr::Value(vws) = inner.as_ref()
            && let Some(p) = value_is_placeholder(vws)
            && let Some(pos) = parse_placeholder(p)
        {
            let neutral = datatype_to_neutral(data_type, self.catalog);
            self.register_param(pos, Some(name.to_string()), Some(neutral), false);
        }
    }

    fn collect_param_from_expr_with_type(&mut self, expr: &Expr, type_str: &str, name: &str) {
        if let Expr::Value(vws) = expr {
            if let Some(p) = value_is_placeholder(vws)
                && let Some(pos) = parse_placeholder(p)
            {
                self.register_param(
                    pos,
                    Some(name.to_string()),
                    Some(type_str.to_string()),
                    false,
                );
            }
        } else if let Expr::Cast {
            expr: inner,
            data_type,
            ..
        } = expr
            && let Expr::Value(vws) = inner.as_ref()
            && let Some(p) = value_is_placeholder(vws)
            && let Some(pos) = parse_placeholder(p)
        {
            let neutral = datatype_to_neutral(data_type, self.catalog);
            self.register_param(pos, Some(name.to_string()), Some(neutral), false);
        }
    }

    fn collect_param_type_from_cast(&mut self, expr: &Expr, neutral_type: &str) {
        if let Expr::Value(vws) = expr
            && let Some(p) = value_is_placeholder(vws)
            && let Some(pos) = parse_placeholder(p)
        {
            // Give semantic names for certain types
            let name = match neutral_type {
                "interval" => Some("duration".to_string()),
                _ => None,
            };
            self.register_param(pos, name, Some(neutral_type.to_string()), false);
        }
    }

    fn collect_param_from_any(&mut self, expr: &Expr, left_ti: &TypeInfo, left_name: &str) {
        match expr {
            Expr::Cast {
                expr: inner,
                data_type,
                ..
            } => {
                if let Expr::Value(vws) = inner.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = parse_placeholder(p)
                {
                    let neutral = datatype_to_neutral(data_type, self.catalog);
                    self.register_param(pos, None, Some(neutral), false);
                }
            }
            Expr::Nested(inner) => self.collect_param_from_any(inner, left_ti, left_name),
            Expr::Array(arr) => {
                // Handle ARRAY[$1, $2, $3] - extract params from array elements
                for (i, elem) in arr.elem.iter().enumerate() {
                    if let Expr::Value(vws) = elem
                        && let Some(p) = value_is_placeholder(vws)
                        && let Some(pos) = parse_placeholder(p)
                    {
                        let name = format!("{}{}", left_name, i + 1);
                        self.register_param(
                            pos,
                            Some(name),
                            Some(left_ti.neutral_type.clone()),
                            false,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // INSERT
    // -----------------------------------------------------------------------

    fn analyze_insert(
        &mut self,
        insert: &ast::Insert,
    ) -> Result<(Vec<AnalyzedColumn>, Vec<ParamInfo>), ScytheError> {
        let table_name = match &insert.table {
            ast::TableObject::TableName(name) => object_name_to_string(name).to_lowercase(),
            ast::TableObject::TableFunction(func) => {
                object_name_to_string(&func.name).to_lowercase()
            }
        };

        let target_cols: Vec<String> = insert
            .columns
            .iter()
            .map(|ident| ident.value.to_lowercase())
            .collect();

        // Collect params from the source (VALUES or subquery)
        if let Some(ref source) = insert.source {
            self.collect_insert_params(&table_name, &target_cols, &source.body)?;
        }

        // Handle ON CONFLICT ... DO UPDATE SET
        if let Some(ref on_conflict) = insert.on
            && let ast::OnInsert::OnConflict(oc) = on_conflict
            && let ast::OnConflictAction::DoUpdate(do_update) = &oc.action
        {
            let scope = self.build_scope_for_table(&table_name)?;
            for assign in &do_update.assignments {
                let col_name = assignment_target_name(&assign.target);
                if let Some(col_type) = self.get_column_type(&table_name, &col_name) {
                    self.collect_param_from_expr_with_type(&assign.value, &col_type, &col_name);
                }
            }
            if let Some(ref selection) = do_update.selection {
                self.collect_params_from_where(selection, &scope);
            }
        }

        // Handle RETURNING clause
        let columns = if let Some(ref returning) = insert.returning {
            self.analyze_returning(&table_name, returning)?
        } else {
            Vec::new()
        };

        Ok((columns, self.params.clone()))
    }

    fn collect_insert_params(
        &mut self,
        table_name: &str,
        target_cols: &[String],
        source: &SetExpr,
    ) -> Result<(), ScytheError> {
        match source {
            SetExpr::Values(values) => {
                for row in &values.rows {
                    for (i, expr) in row.iter().enumerate() {
                        if i < target_cols.len() {
                            let col_name = &target_cols[i];
                            if let Some(col_type) = self.get_column_type(table_name, col_name) {
                                self.collect_param_from_expr_with_type(expr, &col_type, col_name);
                            }
                        }
                    }
                }
            }
            SetExpr::Select(select) => {
                let _ = self.analyze_select(select)?;
            }
            SetExpr::Query(query) => {
                let _ = self.analyze_query(query)?;
            }
            _ => {}
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // UPDATE
    // -----------------------------------------------------------------------

    fn analyze_update(
        &mut self,
        table: &TableWithJoins,
        assignments: &[ast::Assignment],
        from: &Option<ast::UpdateTableFromKind>,
        selection: &Option<Expr>,
        returning: &Option<Vec<SelectItem>>,
    ) -> Result<(Vec<AnalyzedColumn>, Vec<ParamInfo>), ScytheError> {
        let table_name = table_factor_name(&table.relation);

        let mut scope = self.build_scope_for_table(&table_name)?;
        if let Some(from_kind) = from {
            let tables = match from_kind {
                ast::UpdateTableFromKind::BeforeSet(tables)
                | ast::UpdateTableFromKind::AfterSet(tables) => tables,
            };
            let from_scope = self.build_scope_from_from(tables)?;
            scope.sources.extend(from_scope.sources);
        }

        // Collect params from SET clause
        for assign in assignments {
            let col_name = assignment_target_name(&assign.target);
            if let Some(col_type) = self.get_column_type(&table_name, &col_name) {
                self.collect_param_from_expr_with_type(&assign.value, &col_type, &col_name);
            }
        }

        // Collect params from WHERE
        if let Some(sel) = selection {
            self.collect_params_from_where(sel, &scope);
        }

        // Handle RETURNING
        let columns = if let Some(returning) = returning {
            self.analyze_returning(&table_name, returning)?
        } else {
            Vec::new()
        };

        Ok((columns, self.params.clone()))
    }

    // -----------------------------------------------------------------------
    // DELETE
    // -----------------------------------------------------------------------

    fn analyze_delete(
        &mut self,
        delete: &ast::Delete,
    ) -> Result<(Vec<AnalyzedColumn>, Vec<ParamInfo>), ScytheError> {
        let table_name = match &delete.from {
            ast::FromTable::WithFromKeyword(tables) | ast::FromTable::WithoutKeyword(tables) => {
                if let Some(twj) = tables.first() {
                    table_factor_name(&twj.relation)
                } else {
                    String::new()
                }
            }
        };

        let scope = self.build_scope_for_table(&table_name)?;

        let mut full_scope = scope;
        if let Some(ref using) = delete.using {
            let using_scope = self.build_scope_from_from(using)?;
            full_scope.sources.extend(using_scope.sources);
        }

        // Collect params from WHERE
        if let Some(ref selection) = delete.selection {
            self.collect_params_from_where(selection, &full_scope);
        }

        // Handle RETURNING
        let columns = if let Some(ref returning) = delete.returning {
            self.analyze_returning(&table_name, returning)?
        } else {
            Vec::new()
        };

        Ok((columns, self.params.clone()))
    }

    // -----------------------------------------------------------------------
    // RETURNING clause
    // -----------------------------------------------------------------------

    fn analyze_returning(
        &mut self,
        table_name: &str,
        returning: &[SelectItem],
    ) -> Result<Vec<AnalyzedColumn>, ScytheError> {
        let scope = self.build_scope_for_table(table_name)?;
        let mut columns = Vec::new();

        for item in returning {
            match item {
                SelectItem::UnnamedExpr(expr) => {
                    let ti = self.infer_expr_type(expr, &scope);
                    let name = expr_to_name(expr);
                    columns.push(AnalyzedColumn {
                        name,
                        neutral_type: ti.neutral_type,
                        nullable: ti.nullable,
                    });
                }
                SelectItem::ExprWithAlias { expr, alias } => {
                    let ti = self.infer_expr_type(expr, &scope);
                    columns.push(AnalyzedColumn {
                        name: alias.value.to_lowercase(),
                        neutral_type: ti.neutral_type,
                        nullable: ti.nullable,
                    });
                }
                SelectItem::Wildcard(_) => {
                    for source in &scope.sources {
                        for col in &source.columns {
                            columns.push(AnalyzedColumn {
                                name: col.name.clone(),
                                neutral_type: col.neutral_type.clone(),
                                nullable: col.base_nullable,
                            });
                        }
                    }
                }
                SelectItem::QualifiedWildcard(kind, _) => {
                    let qualifier = match kind {
                        ast::SelectItemQualifiedWildcardKind::ObjectName(name) => {
                            object_name_to_string(name).to_lowercase()
                        }
                        ast::SelectItemQualifiedWildcardKind::Expr(expr) => expr_to_name(expr),
                    };
                    for source in &scope.sources {
                        if source.alias == qualifier || source.table_name == qualifier {
                            for col in &source.columns {
                                columns.push(AnalyzedColumn {
                                    name: col.name.clone(),
                                    neutral_type: col.neutral_type.clone(),
                                    nullable: col.base_nullable,
                                });
                            }
                        }
                    }
                }
            }
        }

        Ok(columns)
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn build_scope_for_table(&self, table_name: &str) -> Result<Scope, ScytheError> {
        let mut scope = Scope {
            sources: Vec::new(),
        };

        // Check CTEs first
        if let Some((_, cte_cols)) = self.ctes.iter().find(|(n, _)| n == table_name) {
            scope.sources.push(ScopeSource {
                alias: table_name.to_string(),
                table_name: table_name.to_string(),
                columns: cte_cols.clone(),
                nullable_from_join: false,
            });
            return Ok(scope);
        }

        if let Some(table) = self.catalog.get_table(table_name) {
            let scope_cols: Vec<ScopeColumn> = table
                .columns
                .iter()
                .map(|c| ScopeColumn {
                    name: c.name.clone(),
                    neutral_type: sql_type_to_neutral(&c.sql_type, self.catalog),
                    base_nullable: c.nullable,
                })
                .collect();
            let bare = table_name
                .rsplit_once('.')
                .map(|(_, n)| n.to_string())
                .unwrap_or_else(|| table_name.to_string());
            scope.sources.push(ScopeSource {
                alias: bare,
                table_name: table_name.to_string(),
                columns: scope_cols,
                nullable_from_join: false,
            });
        }

        Ok(scope)
    }

    fn get_column_type(&self, table_name: &str, col_name: &str) -> Option<String> {
        if let Some(table) = self.catalog.get_table(table_name)
            && let Some(col) = table.columns.iter().find(|c| c.name == col_name)
        {
            return Some(sql_type_to_neutral(&col.sql_type, self.catalog));
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Free-standing helpers
// ---------------------------------------------------------------------------

fn object_name_to_string(name: &ObjectName) -> String {
    name.0
        .iter()
        .map(|part| match part {
            ast::ObjectNamePart::Identifier(ident) => ident.value.clone(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn table_factor_name(tf: &TableFactor) -> String {
    match tf {
        TableFactor::Table { name, .. } => object_name_to_string(name).to_lowercase(),
        _ => String::new(),
    }
}

fn assignment_target_name(target: &ast::AssignmentTarget) -> String {
    match target {
        ast::AssignmentTarget::ColumnName(name) => object_name_to_string(name).to_lowercase(),
        ast::AssignmentTarget::Tuple(names) => {
            // Use the first name in the tuple
            names
                .first()
                .map(|n| object_name_to_string(n).to_lowercase())
                .unwrap_or_default()
        }
    }
}

fn expr_to_name(expr: &Expr) -> String {
    match expr {
        Expr::Identifier(ident) => {
            if ident.quote_style.is_some() {
                ident.value.clone()
            } else {
                ident.value.to_lowercase()
            }
        }
        Expr::CompoundIdentifier(parts) => parts
            .last()
            .map(|i| i.value.to_lowercase())
            .unwrap_or_else(|| "unknown".to_string()),
        Expr::Function(func) => object_name_to_string(&func.name).to_lowercase(),
        Expr::Cast { expr: inner, .. } => expr_to_name(inner),
        Expr::Nested(inner) => expr_to_name(inner),
        Expr::UnaryOp { expr: inner, .. } => expr_to_name(inner),
        Expr::Case { .. } => "case".to_string(),
        Expr::Subquery(_) => "subquery".to_string(),
        Expr::Value(vws) => {
            if let Some(p) = value_is_placeholder(vws)
                && let Some(pos) = parse_placeholder(p)
            {
                return format!("p{}", pos);
            }
            "unknown".to_string()
        }
        Expr::BinaryOp { left, .. } => expr_to_name(left),
        Expr::CompoundFieldAccess { access_chain, .. } => {
            // Return the last field name, e.g. (home_address).street -> "street"
            if let Some(last) = access_chain.last()
                && let ast::AccessExpr::Dot(inner) = last
            {
                return expr_to_name(inner);
            }
            "unknown".to_string()
        }
        _ => "unknown".to_string(),
    }
}

/// Detect if a statement is a SELECT * from a single table (for model struct reuse)
fn detect_select_star_source(stmt: &Statement) -> Option<String> {
    if let Statement::Query(query) = stmt
        && let SetExpr::Select(select) = query.body.as_ref()
    {
        // Check: single wildcard projection, single FROM table, no joins
        let is_star =
            select.projection.len() == 1 && matches!(select.projection[0], SelectItem::Wildcard(_));
        if is_star
            && select.from.len() == 1
            && select.from[0].joins.is_empty()
            && let TableFactor::Table { name, .. } = &select.from[0].relation
        {
            let table_name = object_name_to_string(name).to_lowercase();
            return Some(table_name);
        }
    }
    None
}

fn parse_placeholder(s: &str) -> Option<i64> {
    s.strip_prefix('$')?.parse::<i64>().ok()
}

/// Derive a param name from a comparison context.
/// For aggregate functions in HAVING (e.g., COUNT(*) > $1), adds a semantic prefix.
fn derive_param_name_from_comparison(
    col_name: &str,
    col_side: &Expr,
    _param_side: &Expr,
    op: Option<&BinaryOperator>,
) -> String {
    // If the column side is an aggregate function, use min_/max_ prefix
    if let Expr::Function(_) = col_side
        && let Some(op) = op
    {
        match op {
            // col > $1 means param is a minimum for col
            BinaryOperator::Gt | BinaryOperator::GtEq => {
                return format!("min_{}", col_name);
            }
            // col < $1 means param is a maximum for col
            BinaryOperator::Lt | BinaryOperator::LtEq => {
                return format!("max_{}", col_name);
            }
            _ => {}
        }
    }
    col_name.to_string()
}

/// Check if a CASE WHEN condition guards the result from being null.
/// e.g., `WHEN bio IS NOT NULL THEN bio` - the IS NOT NULL condition guarantees bio is non-null.
fn is_not_null_guard(condition: &Expr, result: &Expr) -> bool {
    match condition {
        Expr::IsNotNull(inner) => expr_to_name(inner) == expr_to_name(result),
        _ => false,
    }
}

fn is_literal(expr: &Expr) -> bool {
    match expr {
        Expr::Value(vws) => {
            matches!(
                &vws.value,
                Value::Number(_, _)
                    | Value::SingleQuotedString(_)
                    | Value::DoubleQuotedString(_)
                    | Value::Boolean(_)
            )
        }
        _ => false,
    }
}

/// Simple pluralization: add 's' to a name
fn pluralize(name: &str) -> String {
    if name.ends_with('s') || name.ends_with('x') || name.ends_with("sh") || name.ends_with("ch") {
        format!("{}es", name)
    } else if name.ends_with('y')
        && !name.ends_with("ey")
        && !name.ends_with("ay")
        && !name.ends_with("oy")
    {
        format!("{}ies", &name[..name.len() - 1])
    } else {
        format!("{}s", name)
    }
}

fn is_integer_type(t: &str) -> bool {
    matches!(t, "int16" | "int32" | "int64")
}

fn is_comparable_types(a: &str, b: &str) -> bool {
    // Numeric types are comparable with each other
    let numeric = ["int16", "int32", "int64", "float32", "float64", "decimal"];
    if numeric.contains(&a) && numeric.contains(&b) {
        return true;
    }
    // String types
    if a == "string" && b == "string" {
        return true;
    }
    // Temporal types
    let temporal = ["date", "datetime", "datetime_tz", "time", "time_tz"];
    if temporal.contains(&a) && temporal.contains(&b) {
        return true;
    }
    false
}

fn neutral_to_sql_label(neutral: &str) -> &str {
    match neutral {
        "int16" => "smallint",
        "int32" => "integer",
        "int64" => "bigint",
        "float32" => "real",
        "float64" => "double precision",
        "decimal" => "numeric",
        "string" => "text",
        "bool" => "boolean",
        "bytes" => "bytea",
        "date" => "date",
        "time" => "time",
        "time_tz" => "timetz",
        "datetime" => "timestamp",
        "datetime_tz" => "timestamptz",
        "interval" => "interval",
        "json" => "json",
        "uuid" => "uuid",
        _ => neutral,
    }
}

/// Widen two numeric types to the wider one for UNION type widening
fn widen_type(a: &str, b: &str) -> String {
    if a == b {
        return a.to_string();
    }
    // Integer widening
    let int_rank = |t: &str| -> Option<u8> {
        match t {
            "int16" => Some(0),
            "int32" => Some(1),
            "int64" => Some(2),
            _ => None,
        }
    };
    if let (Some(ra), Some(rb)) = (int_rank(a), int_rank(b)) {
        return if ra >= rb {
            a.to_string()
        } else {
            b.to_string()
        };
    }
    // Float widening
    let float_rank = |t: &str| -> Option<u8> {
        match t {
            "float32" => Some(0),
            "float64" => Some(1),
            _ => None,
        }
    };
    if let (Some(ra), Some(rb)) = (float_rank(a), float_rank(b)) {
        return if ra >= rb {
            a.to_string()
        } else {
            b.to_string()
        };
    }
    // Int + float -> float64
    if int_rank(a).is_some() && float_rank(b).is_some() {
        return "float64".to_string();
    }
    if float_rank(a).is_some() && int_rank(b).is_some() {
        return "float64".to_string();
    }
    // Default: use left side
    a.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_query;

    fn make_catalog() -> Catalog {
        Catalog::from_ddl(&[
            "CREATE TABLE users (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                email VARCHAR(255) NOT NULL,
                age INTEGER,
                active BOOLEAN NOT NULL DEFAULT true,
                created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                bio TEXT,
                score NUMERIC
            );",
            "CREATE TABLE posts (
                id SERIAL PRIMARY KEY,
                user_id INTEGER NOT NULL REFERENCES users(id),
                title TEXT NOT NULL,
                body TEXT,
                published BOOLEAN NOT NULL DEFAULT false,
                created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
            );",
            "CREATE TABLE comments (
                id SERIAL PRIMARY KEY,
                post_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                body TEXT NOT NULL,
                created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
            );",
        ])
        .unwrap()
    }

    #[test]
    fn test_simple_select() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUser
-- @returns :one
SELECT id, name, email FROM users WHERE id = $1;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns.len(), 3);
        assert_eq!(result.columns[0].name, "id");
        assert_eq!(result.columns[0].neutral_type, "int32");
        assert!(!result.columns[0].nullable);
        assert_eq!(result.columns[1].name, "name");
        assert_eq!(result.columns[1].neutral_type, "string");
        assert_eq!(result.columns[2].name, "email");
        assert_eq!(result.columns[2].neutral_type, "string");

        assert_eq!(result.params.len(), 1);
        assert_eq!(result.params[0].position, 1);
        assert_eq!(result.params[0].neutral_type, "int32");
        assert_eq!(result.params[0].name, "id");
    }

    #[test]
    fn test_select_star() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name ListUsers
-- @returns :many
SELECT * FROM users;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns.len(), 8);
    }

    #[test]
    fn test_left_join_nullability() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name UsersWithPosts
-- @returns :many
SELECT u.id, u.name, p.title, p.body FROM users u LEFT JOIN posts p ON u.id = p.user_id;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns.len(), 4);
        assert!(!result.columns[0].nullable);
        assert!(!result.columns[1].nullable);
        assert!(result.columns[2].nullable);
        assert!(result.columns[3].nullable);
    }

    #[test]
    fn test_aggregate_functions() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name UserStats
-- @returns :one
SELECT COUNT(*) as total, AVG(age) as avg_age, MAX(score) as max_score FROM users;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns.len(), 3);
        assert_eq!(result.columns[0].neutral_type, "int64");
        assert!(!result.columns[0].nullable);
        assert_eq!(result.columns[1].neutral_type, "decimal");
        assert!(result.columns[1].nullable);
        assert!(result.columns[2].nullable);
    }

    #[test]
    fn test_insert_returning() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name CreateUser
-- @returns :one
INSERT INTO users (name, email) VALUES ($1, $2) RETURNING id, name, email;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns.len(), 3);
        assert_eq!(result.columns[0].name, "id");
        assert_eq!(result.columns[0].neutral_type, "int32");

        assert_eq!(result.params.len(), 2);
        assert_eq!(result.params[0].name, "name");
        assert_eq!(result.params[0].neutral_type, "string");
        assert_eq!(result.params[1].name, "email");
        assert_eq!(result.params[1].neutral_type, "string");
    }

    #[test]
    fn test_coalesce_nullability() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetBio
-- @returns :one
SELECT COALESCE(bio, 'No bio') as bio FROM users WHERE id = $1;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns[0].neutral_type, "string");
        assert!(!result.columns[0].nullable);
    }

    #[test]
    fn test_case_expression() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetStatus
-- @returns :many
SELECT name, CASE WHEN active THEN 'active' ELSE 'inactive' END as status FROM users;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns[1].name, "status");
        assert_eq!(result.columns[1].neutral_type, "string");
        assert!(!result.columns[1].nullable);
    }

    #[test]
    fn test_nullif() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetScore
-- @returns :many
SELECT NULLIF(score, 0) as adjusted_score FROM users;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns[0].neutral_type, "decimal");
        assert!(result.columns[0].nullable);
    }

    #[test]
    fn test_cast_expression() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetAgeText
-- @returns :many
SELECT CAST(age AS TEXT) as age_text FROM users;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns[0].neutral_type, "string");
    }

    #[test]
    fn test_annotation_overrides() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUser
-- @returns :one
-- @nullable name
-- @nonnull age
SELECT name, age FROM users WHERE id = $1;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert!(result.columns[0].nullable);
        assert!(!result.columns[1].nullable);
    }
}
