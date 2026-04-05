use sqlparser::ast::ObjectName;

use super::type_normalizer::{bare_name, ident_to_lower, normalize_data_type, object_name_to_key};
use super::{Catalog, Column, Table};

impl Catalog {
    // -----------------------------------------------------------------------
    // CREATE VIEW / CREATE MATERIALIZED VIEW
    // -----------------------------------------------------------------------

    pub(super) fn process_create_view(
        &mut self,
        name: ObjectName,
        view_columns: Vec<sqlparser::ast::ViewColumnDef>,
        query: sqlparser::ast::Query,
        _materialized: bool,
    ) -> Result<(), crate::errors::ScytheError> {
        let view_key = object_name_to_key(&name);

        // If explicit column list is provided with types, use those
        if !view_columns.is_empty() {
            let columns: Vec<Column> = view_columns
                .iter()
                .map(|vc| {
                    let sql_type = vc
                        .data_type
                        .as_ref()
                        .map(|dt| {
                            let (t, _) = normalize_data_type(dt, &self.domains);
                            t
                        })
                        .unwrap_or_else(|| "unknown".to_string());
                    Column {
                        name: ident_to_lower(&vc.name),
                        sql_type,
                        nullable: true,
                        default: None,
                        primary_key: false,
                    }
                })
                .collect();
            self.tables.insert(view_key, Table { columns });
            return Ok(());
        }

        // Try to resolve column types from the view's query
        let columns = self.resolve_view_columns(&query);
        self.tables.insert(view_key, Table { columns });
        Ok(())
    }

    /// Resolve columns from a view's SELECT query by matching against known tables.
    pub(super) fn resolve_view_columns(&self, query: &sqlparser::ast::Query) -> Vec<Column> {
        use sqlparser::ast::{SelectItem, SetExpr, TableFactor};

        if let SetExpr::Select(select) = query.body.as_ref() {
            // Build a map of alias/table -> columns from FROM clause
            let mut source_cols: Vec<(String, String, Vec<Column>)> = Vec::new(); // (alias, table_name, columns)
            for twj in &select.from {
                if let TableFactor::Table { name, alias, .. } = &twj.relation {
                    let table_name = object_name_to_key(name);
                    let alias_name = alias
                        .as_ref()
                        .map(|a| a.name.value.to_lowercase())
                        .unwrap_or_else(|| bare_name(&table_name).to_string());
                    if let Some(table) = self
                        .tables
                        .get(&table_name)
                        .or_else(|| self.tables.get(bare_name(&table_name)))
                    {
                        source_cols.push((alias_name, table_name, table.columns.clone()));
                    }
                }
                for join in &twj.joins {
                    if let TableFactor::Table { name, alias, .. } = &join.relation {
                        let table_name = object_name_to_key(name);
                        let alias_name = alias
                            .as_ref()
                            .map(|a| a.name.value.to_lowercase())
                            .unwrap_or_else(|| bare_name(&table_name).to_string());
                        if let Some(table) = self
                            .tables
                            .get(&table_name)
                            .or_else(|| self.tables.get(bare_name(&table_name)))
                        {
                            source_cols.push((alias_name, table_name, table.columns.clone()));
                        }
                    }
                }
            }

            let mut result = Vec::new();
            for item in &select.projection {
                match item {
                    SelectItem::UnnamedExpr(expr) => {
                        let (name, sql_type, nullable) = self.resolve_view_expr(expr, &source_cols);
                        result.push(Column {
                            name,
                            sql_type,
                            nullable,
                            default: None,
                            primary_key: false,
                        });
                    }
                    SelectItem::ExprWithAlias { expr, alias } => {
                        let (_, sql_type, nullable) = self.resolve_view_expr(expr, &source_cols);
                        result.push(Column {
                            name: ident_to_lower(alias),
                            sql_type,
                            nullable,
                            default: None,
                            primary_key: false,
                        });
                    }
                    SelectItem::Wildcard(_) => {
                        for (_, _, cols) in &source_cols {
                            for col in cols {
                                result.push(col.clone());
                            }
                        }
                    }
                    SelectItem::QualifiedWildcard(kind, _) => {
                        let qualifier = match kind {
                            sqlparser::ast::SelectItemQualifiedWildcardKind::ObjectName(name) => {
                                object_name_to_key(name)
                            }
                            _ => String::new(),
                        };
                        for (alias, tname, cols) in &source_cols {
                            if *alias == qualifier || *tname == qualifier {
                                for col in cols {
                                    result.push(col.clone());
                                }
                            }
                        }
                    }
                }
            }
            if !result.is_empty() {
                return result;
            }
        }
        Vec::new()
    }

    /// Resolve a single expression in a view SELECT to (name, sql_type, nullable)
    pub(super) fn resolve_view_expr(
        &self,
        expr: &sqlparser::ast::Expr,
        sources: &[(String, String, Vec<Column>)],
    ) -> (String, String, bool) {
        use sqlparser::ast::Expr;
        match expr {
            Expr::Identifier(ident) => {
                let col_name = ident_to_lower(ident);
                for (_, _, cols) in sources {
                    if let Some(col) = cols.iter().find(|c| c.name == col_name) {
                        return (col_name, col.sql_type.clone(), col.nullable);
                    }
                }
                (col_name, "unknown".to_string(), true)
            }
            Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                let qualifier = parts[0].value.to_lowercase();
                let col_name = ident_to_lower(&parts[1]);
                for (alias, _, cols) in sources {
                    if *alias == qualifier
                        && let Some(col) = cols.iter().find(|c| c.name == col_name)
                    {
                        return (col_name, col.sql_type.clone(), col.nullable);
                    }
                }
                (col_name, "unknown".to_string(), true)
            }
            Expr::Function(func) => {
                let func_name = object_name_to_key(&func.name).to_lowercase();
                let name = func_name.clone();
                match func_name.as_str() {
                    "count" => (name, "bigint".to_string(), false),
                    "sum" => (name, "bigint".to_string(), true),
                    "avg" => (name, "numeric".to_string(), true),
                    "min" | "max" => {
                        // Try to get arg type
                        if let sqlparser::ast::FunctionArguments::List(args) = &func.args
                            && let Some(first) = args.args.first()
                            && let sqlparser::ast::FunctionArg::Unnamed(
                                sqlparser::ast::FunctionArgExpr::Expr(e),
                            ) = first
                        {
                            let (_, t, _) = self.resolve_view_expr(e, sources);
                            return (name, t, true);
                        }
                        (name, "unknown".to_string(), true)
                    }
                    _ => (name, "unknown".to_string(), true),
                }
            }
            _ => ("unknown".to_string(), "unknown".to_string(), true),
        }
    }
}
