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

#[cfg(test)]
mod tests {
    use crate::catalog::Catalog;

    #[test]
    fn test_simple_view() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL, status TEXT);",
            "CREATE VIEW active_users AS SELECT id, name FROM users WHERE status = 'active';",
        ])
        .unwrap();
        let view = catalog
            .get_table("active_users")
            .expect("view should exist");
        assert_eq!(view.columns.len(), 2);
        assert_eq!(view.columns[0].name, "id");
        assert_eq!(view.columns[0].sql_type, "integer");
        assert_eq!(view.columns[1].name, "name");
        assert_eq!(view.columns[1].sql_type, "text");
    }

    #[test]
    fn test_view_with_join() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL);",
            "CREATE TABLE orders (id SERIAL PRIMARY KEY, user_id INTEGER NOT NULL, total NUMERIC(10,2));",
            "CREATE VIEW user_orders AS SELECT u.id, u.name, o.total FROM users u JOIN orders o ON u.id = o.user_id;",
        ])
        .unwrap();
        let view = catalog.get_table("user_orders").expect("view should exist");
        assert_eq!(view.columns.len(), 3);
        assert_eq!(view.columns[0].name, "id");
        assert_eq!(view.columns[0].sql_type, "integer");
        assert_eq!(view.columns[1].name, "name");
        assert_eq!(view.columns[1].sql_type, "text");
        assert_eq!(view.columns[2].name, "total");
        assert_eq!(view.columns[2].sql_type, "numeric(10,2)");
    }

    #[test]
    fn test_view_with_alias() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL);",
            "CREATE VIEW aliased AS SELECT id AS user_id FROM users;",
        ])
        .unwrap();
        let view = catalog.get_table("aliased").expect("view should exist");
        assert_eq!(view.columns.len(), 1);
        assert_eq!(view.columns[0].name, "user_id");
        assert_eq!(view.columns[0].sql_type, "integer");
    }

    #[test]
    fn test_view_with_star() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL, email VARCHAR(255));",
            "CREATE VIEW all_users AS SELECT * FROM users;",
        ])
        .unwrap();
        let view = catalog.get_table("all_users").expect("view should exist");
        assert_eq!(view.columns.len(), 3);
        assert_eq!(view.columns[0].name, "id");
        assert_eq!(view.columns[1].name, "name");
        assert_eq!(view.columns[2].name, "email");
        assert_eq!(view.columns[2].sql_type, "varchar(255)");
    }

    #[test]
    fn test_materialized_view() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL, status TEXT);",
            "CREATE MATERIALIZED VIEW mv AS SELECT id, name FROM users WHERE status = 'active';",
        ])
        .unwrap();
        let view = catalog
            .get_table("mv")
            .expect("materialized view should exist");
        assert_eq!(view.columns.len(), 2);
        assert_eq!(view.columns[0].name, "id");
        assert_eq!(view.columns[1].name, "name");
    }

    #[test]
    fn test_view_from_view() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL, status TEXT);",
            "CREATE VIEW active_users AS SELECT id, name FROM users WHERE status = 'active';",
            "CREATE VIEW active_names AS SELECT name FROM active_users;",
        ])
        .unwrap();
        let view = catalog
            .get_table("active_names")
            .expect("view-from-view should exist");
        assert_eq!(view.columns.len(), 1);
        assert_eq!(view.columns[0].name, "name");
        assert_eq!(view.columns[0].sql_type, "text");
    }
}
