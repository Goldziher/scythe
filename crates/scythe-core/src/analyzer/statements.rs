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

    // -----------------------------------------------------------------------
    // SELECT / Query
    // -----------------------------------------------------------------------

    pub(super) fn analyze_query(
        &mut self,
        query: &ast::Query,
    ) -> Result<Vec<AnalyzedColumn>, ScytheError> {
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
                            self.ctes.insert(cte_name.clone(), scope_cols);
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
                self.ctes.insert(cte_name, scope_cols);
            }
        }

        // Analyze the query body (collects params from WHERE/HAVING/projections)
        let result = self.analyze_set_expr(&query.body)?;

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

        Ok(result)
    }

    pub(super) fn analyze_set_expr(
        &mut self,
        set_expr: &SetExpr,
    ) -> Result<Vec<AnalyzedColumn>, ScytheError> {
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

    pub(super) fn analyze_select(
        &mut self,
        select: &ast::Select,
    ) -> Result<Vec<AnalyzedColumn>, ScytheError> {
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

        // 4. Resolve select items (also collect params from expressions)
        let mut columns = Vec::new();
        for item in &select.projection {
            match item {
                SelectItem::UnnamedExpr(expr) => {
                    self.collect_params_from_where(expr, &scope);
                    let ti = self.infer_expr_type(expr, &scope);
                    let name = expr_to_name(expr);
                    columns.push(AnalyzedColumn {
                        name,
                        neutral_type: ti.neutral_type,
                        nullable: ti.nullable,
                    });
                }
                SelectItem::ExprWithAlias { expr, alias } => {
                    self.collect_params_from_where(expr, &scope);
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
        let mut seen_names: AHashSet<String> = AHashSet::new();
        for col in &columns {
            if !seen_names.insert(col.name.clone()) {
                return Err(ScytheError::duplicate_alias(&col.name));
            }
        }

        Ok(columns)
    }

    // -----------------------------------------------------------------------
    // INSERT
    // -----------------------------------------------------------------------

    pub(super) fn analyze_insert(
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
}
