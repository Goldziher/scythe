use ahash::AHashSet;
use sqlparser::ast::{self, Expr, SelectItem, SetExpr, Statement};

use crate::errors::ScytheError;

use super::helpers::*;
use super::types::*;

impl<'a> Analyzer<'a> {
    pub(super) fn analyze_statement(
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

    pub(super) fn analyze_query(&mut self, query: &ast::Query) -> Result<Vec<AnalyzedColumn>, ScytheError> {
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                let cte_name = cte.alias.name.value.to_lowercase();
                if with.recursive {
                    let is_union = matches!(cte.query.body.as_ref(), SetExpr::SetOperation { .. });
                    if !is_union {
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
                        if let SetExpr::SetOperation { left, .. } = cte.query.body.as_ref() {
                            // The recursive term references the CTE by name, so the
                            // anchor's own types must be in scope before it can be
                            // analyzed: seed `self.ctes` with the anchor-only shape
                            // first.
                            let base_cols = self.analyze_set_expr(left)?;
                            let scope_cols: Vec<ScopeColumn> = base_cols
                                .iter()
                                .map(|c| {
                                    ScopeColumn::from_catalog(
                                        c.name.clone(),
                                        c.sql_type.clone(),
                                        c.neutral_type.clone(),
                                        c.nullable,
                                    )
                                })
                                .collect();
                            self.ctes.insert(cte_name.clone(), scope_cols);

                            // Re-analyze the full anchor-UNION-recursive query and
                            // keep the result instead of discarding it. That takes
                            // the SetOperation path in analyze_set_expr, which
                            // widens nullability across both branches and errors on
                            // a column-count mismatch between them — the same rules
                            // as any other UNION. Anchor-only typing under-reports
                            // nullability whenever the recursive term introduces a
                            // NULL the anchor doesn't have, e.g. a LEFT JOIN or an
                            // explicit NULL literal in a position the anchor fills
                            // with a NOT NULL column.
                            let full_cols = self.analyze_query(&cte.query)?;
                            let widened_scope_cols: Vec<ScopeColumn> = full_cols
                                .iter()
                                .map(|c| {
                                    ScopeColumn::from_catalog(
                                        c.name.clone(),
                                        c.sql_type.clone(),
                                        c.neutral_type.clone(),
                                        c.nullable,
                                    )
                                })
                                .collect();
                            self.ctes.insert(cte_name.clone(), widened_scope_cols);
                            continue;
                        }
                    }
                }
                let cte_cols = self.analyze_query(&cte.query)?;
                let scope_cols: Vec<ScopeColumn> = cte_cols
                    .iter()
                    .map(|c| {
                        ScopeColumn::from_catalog(
                            c.name.clone(),
                            c.sql_type.clone(),
                            c.neutral_type.clone(),
                            c.nullable,
                        )
                    })
                    .collect();
                self.ctes.insert(cte_name, scope_cols);
            }
        }

        let result = self.analyze_set_expr(&query.body)?;

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

        Ok(result)
    }

    pub(super) fn analyze_set_expr(&mut self, set_expr: &SetExpr) -> Result<Vec<AnalyzedColumn>, ScytheError> {
        match set_expr {
            SetExpr::Select(select) => self.analyze_select(select),
            SetExpr::Query(query) => self.analyze_query(query),
            SetExpr::SetOperation { left, right, .. } => {
                let left_cols = self.analyze_set_expr(left)?;
                let right_cols = self.analyze_set_expr(right)?;
                if !left_cols.is_empty() && !right_cols.is_empty() && left_cols.len() != right_cols.len() {
                    return Err(ScytheError::column_count_mismatch(left_cols.len(), right_cols.len()));
                }
                let widened: Vec<AnalyzedColumn> = left_cols
                    .iter()
                    .enumerate()
                    .map(|(i, lc)| {
                        if i < right_cols.len() {
                            let widened_type = self.widen_union_arm_type(&lc.neutral_type, &right_cols[i].neutral_type);
                            // Preserve the source `sql_type` (e.g. "clob") when both sides
                            // of the UNION agree on it — backends like rust-sibyl match on
                            // `sql_type`, not `neutral_type`, to detect DB-specific column
                            // kinds (CLOB/NCLOB/BLOB/BFILE) that need non-default handling.
                            // Only fall back to the widened neutral type when the sides
                            // genuinely disagree on the source type.
                            let sql_type = if lc.sql_type == right_cols[i].sql_type {
                                lc.sql_type.clone()
                            } else {
                                widened_type.clone()
                            };
                            AnalyzedColumn {
                                name: lc.name.clone(),
                                sql_type,
                                neutral_type: widened_type,
                                nullable: lc.nullable || right_cols[i].nullable,
                                ..Default::default()
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
                            let ti = self.infer_expr_type(expr, &Scope { sources: Vec::new() });
                            AnalyzedColumn::from_type_info(format!("column{}", i + 1), ti)
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

    /// Widen two UNION-arm column types, with a nested-aggregate-aware path
    /// `widen_type` alone can't provide.
    ///
    /// `widen_type` has no concept of `json_typed<...>`, and every
    /// `json_agg`/`row_to_json` call gets a *fresh* `__nested__{id}`
    /// placeholder (see `Analyzer::push_pending_nested`) -- so two arms
    /// producing the exact same nested shape would still carry different
    /// ids and never compare equal there. Left unhandled, `widen_type`'s
    /// existing "genuinely different types -> left wins" fallback would
    /// silently discard whichever arm's shape didn't happen to be on the
    /// left, even when the two arms describe different underlying rows --
    /// this is the one case where "left silently wins" is not an
    /// acceptable degradation, because the discarded shape's fields are
    /// gone with no diagnostic.
    ///
    /// Only one path recurses through `self.pending_nested`; every other
    /// input (including a nested type paired with a non-nested one, which
    /// is an ordinary type mismatch the same as any other) falls straight
    /// through to `widen_type` unchanged.
    fn widen_union_arm_type(&mut self, left: &str, right: &str) -> String {
        let (Some(left_id), Some(right_id)) = (find_nested_placeholder_id(left), find_nested_placeholder_id(right))
        else {
            return widen_type(left, right);
        };

        let left_fields = self
            .pending_nested
            .iter()
            .find(|p| p.id == left_id)
            .map(|p| p.fields.clone());
        let right_fields = self
            .pending_nested
            .iter()
            .find(|p| p.id == right_id)
            .map(|p| p.fields.clone());

        if left_fields.is_some() && left_fields == right_fields {
            // Both arms produced the identical nested shape (same fields,
            // same order, same types and nullability) -- interchangeable,
            // so keeping the left arm's placeholder (and letting phase 2
            // resolve just that one) is correct, not merely convenient.
            return left.to_string();
        }

        self.type_errors.push(
            "UNION arms both produce a nested aggregate (json_agg/row_to_json) but with different row \
             shapes; a UNION requires every arm to produce the same column types, and there is no way to \
             widen two different nested struct shapes into one"
                .to_string(),
        );
        left.to_string()
    }

    pub(super) fn analyze_select(&mut self, select: &ast::Select) -> Result<Vec<AnalyzedColumn>, ScytheError> {
        let scope = self.build_scope_from_from(&select.from)?;

        if let Some(ref selection) = select.selection {
            self.collect_params_from_where(selection, &scope);
        }

        if let Some(ref having) = select.having {
            self.collect_params_from_where(having, &scope);
        }

        let mut columns = Vec::new();
        for item in &select.projection {
            match item {
                SelectItem::UnnamedExpr(expr) => {
                    self.collect_params_from_where(expr, &scope);
                    let ti = self.infer_expr_type(expr, &scope);
                    let name = expr_to_name(expr);
                    columns.push(AnalyzedColumn::from_type_info(name, ti));
                }
                SelectItem::ExprWithAlias { expr, alias } => {
                    self.collect_params_from_where(expr, &scope);
                    let ti = self.infer_expr_type(expr, &scope);
                    columns.push(AnalyzedColumn::from_type_info(alias.value.to_lowercase(), ti));
                }
                SelectItem::ExprWithAliases { expr, aliases } => {
                    self.collect_params_from_where(expr, &scope);
                    let ti = self.infer_expr_type(expr, &scope);
                    for alias in aliases {
                        columns.push(AnalyzedColumn::from_type_info(alias.value.to_lowercase(), ti.clone()));
                    }
                }
                SelectItem::Wildcard(_) => {
                    for source in &scope.sources {
                        for col in &source.columns {
                            columns.push(AnalyzedColumn::from_type_info(
                                col.name.clone(),
                                TypeInfo::from_scope_column(
                                    col.sql_type.clone(),
                                    col.neutral_type.clone(),
                                    col.base_nullable,
                                    &source.alias,
                                    source.nullable_from_join,
                                ),
                            ));
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
                                columns.push(AnalyzedColumn::from_type_info(
                                    col.name.clone(),
                                    TypeInfo::from_scope_column(
                                        col.sql_type.clone(),
                                        col.neutral_type.clone(),
                                        col.base_nullable,
                                        &source.alias,
                                        source.nullable_from_join,
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }

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

        let mut seen_names: AHashSet<String> = AHashSet::new();
        for col in &columns {
            if !seen_names.insert(col.name.clone()) {
                return Err(ScytheError::duplicate_alias(&col.name));
            }
        }

        Ok(columns)
    }

    pub(super) fn analyze_insert(
        &mut self,
        insert: &ast::Insert,
    ) -> Result<(Vec<AnalyzedColumn>, Vec<ParamInfo>), ScytheError> {
        let table_name = match &insert.table {
            ast::TableObject::TableName(name) => object_name_to_string(name).to_lowercase(),
            ast::TableObject::TableFunction(func) => object_name_to_string(&func.name).to_lowercase(),
            ast::TableObject::TableQuery(_) => {
                return Err(ScytheError::syntax(
                    "INSERT target must be a table name or table function",
                ));
            }
        };

        let target_cols: Vec<String> = insert
            .columns
            .iter()
            .map(|name| object_name_to_string(name).to_lowercase())
            .collect();

        if let Some(ref source) = insert.source {
            self.collect_insert_params(&table_name, &target_cols, &source.body)?;
        }

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
                // Resolve the column each VALUES position binds to: the explicit
                // column list, or — when omitted — every column of the table in
                // catalog order (standard SQL positional binding for
                // `INSERT INTO t VALUES (...)`). A position with no known column
                // falls back to inferring the expression's own type, so
                // placeholders are never dropped silently.
                let effective_cols: Vec<Option<(String, String, bool)>> = if target_cols.is_empty() {
                    match self.catalog.get_table(table_name) {
                        Some(table) => table
                            .columns
                            .iter()
                            .map(|c| Some((c.name.clone(), self.get_column_type(table_name, &c.name)?, c.nullable)))
                            .collect(),
                        None => Vec::new(),
                    }
                } else {
                    target_cols
                        .iter()
                        .map(|n| {
                            Some((
                                n.clone(),
                                self.get_column_type(table_name, n)?,
                                self.is_column_nullable(table_name, n),
                            ))
                        })
                        .collect()
                };

                for row in &values.rows {
                    for (i, expr) in row.iter().enumerate() {
                        if let Some((col_name, col_type, nullable)) = effective_cols.get(i).and_then(|c| c.as_ref()) {
                            self.collect_param_from_expr_with_type_nullable(expr, col_type, col_name, *nullable);
                        } else {
                            let ti = self.infer_expr_type(expr, &Scope { sources: Vec::new() });
                            let name = target_cols.get(i).cloned().unwrap_or_else(|| expr_to_name(expr));
                            self.collect_param_from_expr_with_type_nullable(expr, &ti.neutral_type, &name, ti.nullable);
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

    pub(super) fn analyze_update(
        &mut self,
        table: &ast::TableWithJoins,
        assignments: &[ast::Assignment],
        from: &Option<ast::UpdateTableFromKind>,
        selection: &Option<Expr>,
        returning: &Option<Vec<SelectItem>>,
    ) -> Result<(Vec<AnalyzedColumn>, Vec<ParamInfo>), ScytheError> {
        let table_name = table_factor_name(&table.relation);

        let mut scope = self.build_scope_for_table(&table_name)?;
        if let Some(from_kind) = from {
            let tables = match from_kind {
                ast::UpdateTableFromKind::BeforeSet(tables) | ast::UpdateTableFromKind::AfterSet(tables) => tables,
            };
            let from_scope = self.build_scope_from_from(tables)?;
            scope.sources.extend(from_scope.sources);
        }

        for assign in assignments {
            let col_name = assignment_target_name(&assign.target);
            if let Some(col_type) = self.get_column_type(&table_name, &col_name) {
                self.collect_param_from_expr_with_type(&assign.value, &col_type, &col_name);
            }
        }

        if let Some(sel) = selection {
            self.collect_params_from_where(sel, &scope);
        }

        let columns = if let Some(returning) = returning {
            self.analyze_returning(&table_name, returning)?
        } else {
            Vec::new()
        };

        Ok((columns, self.params.clone()))
    }

    pub(super) fn analyze_delete(
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

        if let Some(ref selection) = delete.selection {
            self.collect_params_from_where(selection, &full_scope);
        }

        let columns = if let Some(ref returning) = delete.returning {
            self.analyze_returning(&table_name, returning)?
        } else {
            Vec::new()
        };

        Ok((columns, self.params.clone()))
    }

    pub(super) fn analyze_returning(
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
                    columns.push(AnalyzedColumn::from_type_info(name, ti));
                }
                SelectItem::ExprWithAlias { expr, alias } => {
                    let ti = self.infer_expr_type(expr, &scope);
                    columns.push(AnalyzedColumn::from_type_info(alias.value.to_lowercase(), ti));
                }
                SelectItem::ExprWithAliases { expr, aliases } => {
                    let ti = self.infer_expr_type(expr, &scope);
                    for alias in aliases {
                        columns.push(AnalyzedColumn::from_type_info(alias.value.to_lowercase(), ti.clone()));
                    }
                }
                SelectItem::Wildcard(_) => {
                    for source in &scope.sources {
                        for col in &source.columns {
                            columns.push(AnalyzedColumn {
                                name: col.name.clone(),
                                sql_type: col.sql_type.clone(),
                                neutral_type: col.neutral_type.clone(),
                                nullable: col.base_nullable,
                                ..Default::default()
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
                                    sql_type: col.sql_type.clone(),
                                    neutral_type: col.neutral_type.clone(),
                                    nullable: col.base_nullable,
                                    ..Default::default()
                                });
                            }
                        }
                    }
                }
            }
        }

        Ok(columns)
    }
}

#[cfg(test)]
mod union_sql_type_tests {
    use super::*;
    use crate::catalog::Catalog;
    use ahash::AHashMap;
    use sqlparser::dialect::PostgreSqlDialect;
    use sqlparser::parser::Parser;

    fn make_catalog(ddl: &[&str]) -> Catalog {
        Catalog::from_ddl(ddl).unwrap()
    }

    fn make_analyzer(catalog: &Catalog) -> Analyzer<'_> {
        Analyzer {
            catalog,
            params: Vec::new(),
            ctes: AHashMap::new(),
            type_errors: Vec::new(),
            positional_param_counter: 0,
            pending_nested: Vec::new(),
            next_nested_id: 0,
        }
    }

    fn parse_query(sql: &str) -> ast::Query {
        let dialect = PostgreSqlDialect {};
        let stmts = Parser::parse_sql(&dialect, sql).unwrap();
        let Statement::Query(query) = &stmts[0] else {
            unreachable!("test SQL must be a query");
        };
        (**query).clone()
    }

    /// Regression test for the bug where `SELECT clob_col FROM a UNION SELECT clob_col FROM
    /// b` lost the source `sql_type` ("clob") on the UNION result column, overwriting it
    /// with the *neutral* type ("string"). Backends like rust-sibyl match on `sql_type` to
    /// detect DB-specific column kinds (CLOB) that need non-default handling; losing it
    /// silently reverts a UNION-projected CLOB column to the broken `row.get::<String>`
    /// path. Both sides of the UNION here have identical `sql_type`s, so it must be
    /// preserved verbatim on the result column.
    #[test]
    fn test_union_over_matching_clob_columns_preserves_sql_type() {
        let catalog = make_catalog(&[
            "CREATE TABLE a (id INTEGER, notes CLOB);",
            "CREATE TABLE b (id INTEGER, notes CLOB);",
        ]);
        let mut analyzer = make_analyzer(&catalog);
        let query = parse_query("SELECT id, notes FROM a UNION SELECT id, notes FROM b");

        let columns = analyzer.analyze_query(&query).unwrap();
        let notes = columns
            .iter()
            .find(|c| c.name == "notes")
            .expect("notes column present");

        assert_eq!(
            notes.sql_type, "clob",
            "UNION over a CLOB column on both sides must preserve sql_type == \"clob\" \
             (not fall back to the neutral type \"string\"); got sql_type = {:?}",
            notes.sql_type
        );
        assert_eq!(
            notes.neutral_type, "string",
            "neutral_type must still be the widened neutral type"
        );
    }

    /// When the two sides of a UNION genuinely disagree on source `sql_type` (e.g. `CLOB`
    /// vs `VARCHAR`), there's no single source type to preserve, so the result must fall
    /// back to the widened neutral type rather than picking one side arbitrarily.
    #[test]
    fn test_union_over_mismatched_types_falls_back_to_widened_neutral_type() {
        let catalog = make_catalog(&[
            "CREATE TABLE a (id INTEGER, notes CLOB);",
            "CREATE TABLE b (id INTEGER, notes VARCHAR(255));",
        ]);
        let mut analyzer = make_analyzer(&catalog);
        let query = parse_query("SELECT id, notes FROM a UNION SELECT id, notes FROM b");

        let columns = analyzer.analyze_query(&query).unwrap();
        let notes = columns
            .iter()
            .find(|c| c.name == "notes")
            .expect("notes column present");

        assert_eq!(notes.neutral_type, "string");
        assert_eq!(
            notes.sql_type, "string",
            "mismatched sql_types across a UNION must fall back to the neutral type; got sql_type = {:?}",
            notes.sql_type
        );
    }
}
