use sqlparser::ast::{self, JoinOperator, TableFactor};

use crate::errors::ScytheError;

use super::helpers::object_name_to_string;
use super::type_conversion::sql_type_to_neutral;
use super::types::*;

impl<'a> Analyzer<'a> {
    pub(super) fn build_scope_from_from(
        &self,
        from: &[ast::TableWithJoins],
    ) -> Result<Scope, ScytheError> {
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
                let scope_cols = if let Some(cte_cols) = self.ctes.get(&table_name) {
                    cte_cols.clone()
                } else if let Some(table) = self.catalog.get_table(&table_name) {
                    table
                        .columns
                        .iter()
                        .map(|c| ScopeColumn {
                            name: c.name.clone(),
                            neutral_type: sql_type_to_neutral(&c.sql_type, self.catalog)
                                .into_owned(),
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
                    positional_param_counter: self.positional_param_counter,
                };
                let sub_cols = sub_analyzer.analyze_query(subquery)?;

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

    pub(super) fn build_scope_for_table(&self, table_name: &str) -> Result<Scope, ScytheError> {
        let mut scope = Scope {
            sources: Vec::new(),
        };

        // Check CTEs first
        if let Some(cte_cols) = self.ctes.get(table_name) {
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
                    neutral_type: sql_type_to_neutral(&c.sql_type, self.catalog).into_owned(),
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

    pub(super) fn get_column_type(&self, table_name: &str, col_name: &str) -> Option<String> {
        if let Some(table) = self.catalog.get_table(table_name)
            && let Some(col) = table.columns.iter().find(|c| c.name == col_name)
        {
            return Some(sql_type_to_neutral(&col.sql_type, self.catalog).into_owned());
        }
        None
    }

    pub(super) fn is_column_nullable(&self, table_name: &str, col_name: &str) -> bool {
        if let Some(table) = self.catalog.get_table(table_name)
            && let Some(col) = table.columns.iter().find(|c| c.name == col_name)
        {
            return col.nullable;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use ahash::AHashMap;
    use sqlparser::ast::Statement;
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
        }
    }

    fn parse_from(sql: &str) -> Vec<ast::TableWithJoins> {
        let dialect = PostgreSqlDialect {};
        let stmts = Parser::parse_sql(&dialect, sql).unwrap();
        let Statement::Query(q) = &stmts[0] else {
            unreachable!("test SQL must be a SELECT query");
        };
        let ast::SetExpr::Select(sel) = q.body.as_ref() else {
            unreachable!("test SQL must be a simple SELECT");
        };
        sel.from.clone()
    }

    // -----------------------------------------------------------------------
    // 1. build_scope_for_table
    // -----------------------------------------------------------------------
    #[test]
    fn test_build_scope_for_table_columns_and_types() {
        let catalog = make_catalog(&[
            "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL, email TEXT);",
        ]);
        let analyzer = make_analyzer(&catalog);
        let scope = analyzer.build_scope_for_table("users").unwrap();

        assert_eq!(scope.sources.len(), 1);
        let src = &scope.sources[0];
        assert_eq!(src.alias, "users");
        assert_eq!(src.table_name, "users");
        assert!(!src.nullable_from_join);

        // Check columns
        assert_eq!(src.columns.len(), 3);
        assert_eq!(src.columns[0].name, "id");
        assert!(!src.columns[0].base_nullable); // serial PK
        assert_eq!(src.columns[1].name, "name");
        assert!(!src.columns[1].base_nullable); // NOT NULL
        assert_eq!(src.columns[2].name, "email");
        assert!(src.columns[2].base_nullable); // nullable
    }

    #[test]
    fn test_build_scope_for_table_unknown_returns_empty() {
        let catalog = make_catalog(&["CREATE TABLE users (id INT);"]);
        let analyzer = make_analyzer(&catalog);
        let scope = analyzer.build_scope_for_table("nonexistent").unwrap();
        assert_eq!(scope.sources.len(), 0);
    }

    // -----------------------------------------------------------------------
    // 2. build_scope_from_from — single table
    // -----------------------------------------------------------------------
    #[test]
    fn test_build_scope_from_from_single_table() {
        let catalog = make_catalog(&["CREATE TABLE users (id INT NOT NULL, name TEXT NOT NULL);"]);
        let analyzer = make_analyzer(&catalog);
        let from = parse_from("SELECT * FROM users");
        let scope = analyzer.build_scope_from_from(&from).unwrap();

        assert_eq!(scope.sources.len(), 1);
        let src = &scope.sources[0];
        assert_eq!(src.alias, "users");
        assert_eq!(src.columns.len(), 2);
        assert_eq!(src.columns[0].name, "id");
        assert_eq!(src.columns[1].name, "name");
        assert!(!src.nullable_from_join);
    }

    // -----------------------------------------------------------------------
    // 3. build_scope_from_from with alias
    // -----------------------------------------------------------------------
    #[test]
    fn test_build_scope_from_from_with_alias() {
        let catalog = make_catalog(&["CREATE TABLE users (id INT NOT NULL, name TEXT NOT NULL);"]);
        let analyzer = make_analyzer(&catalog);
        let from = parse_from("SELECT * FROM users u");
        let scope = analyzer.build_scope_from_from(&from).unwrap();

        assert_eq!(scope.sources.len(), 1);
        assert_eq!(scope.sources[0].alias, "u");
        assert_eq!(scope.sources[0].table_name, "users");
    }

    // -----------------------------------------------------------------------
    // 4. INNER JOIN — both sides non-nullable
    // -----------------------------------------------------------------------
    #[test]
    fn test_build_scope_from_from_inner_join() {
        let catalog = make_catalog(&[
            "CREATE TABLE users (id INT NOT NULL);",
            "CREATE TABLE orders (id INT NOT NULL, user_id INT NOT NULL);",
        ]);
        let analyzer = make_analyzer(&catalog);
        let from = parse_from("SELECT * FROM users INNER JOIN orders ON users.id = orders.user_id");
        let scope = analyzer.build_scope_from_from(&from).unwrap();

        assert_eq!(scope.sources.len(), 2);
        assert!(!scope.sources[0].nullable_from_join); // users
        assert!(!scope.sources[1].nullable_from_join); // orders
    }

    // -----------------------------------------------------------------------
    // 5. LEFT JOIN — right side marked nullable
    // -----------------------------------------------------------------------
    #[test]
    fn test_build_scope_from_from_left_join() {
        let catalog = make_catalog(&[
            "CREATE TABLE users (id INT NOT NULL);",
            "CREATE TABLE orders (id INT NOT NULL, user_id INT NOT NULL);",
        ]);
        let analyzer = make_analyzer(&catalog);
        let from = parse_from("SELECT * FROM users LEFT JOIN orders ON users.id = orders.user_id");
        let scope = analyzer.build_scope_from_from(&from).unwrap();

        assert_eq!(scope.sources.len(), 2);
        assert!(!scope.sources[0].nullable_from_join); // users — left side stays non-nullable
        assert!(scope.sources[1].nullable_from_join); // orders — right side becomes nullable
    }

    // -----------------------------------------------------------------------
    // 6. RIGHT JOIN — left side marked nullable
    // -----------------------------------------------------------------------
    #[test]
    fn test_build_scope_from_from_right_join() {
        let catalog = make_catalog(&[
            "CREATE TABLE users (id INT NOT NULL);",
            "CREATE TABLE orders (id INT NOT NULL, user_id INT NOT NULL);",
        ]);
        let analyzer = make_analyzer(&catalog);
        let from = parse_from("SELECT * FROM users RIGHT JOIN orders ON users.id = orders.user_id");
        let scope = analyzer.build_scope_from_from(&from).unwrap();

        assert_eq!(scope.sources.len(), 2);
        assert!(scope.sources[0].nullable_from_join); // users — left side becomes nullable
        assert!(!scope.sources[1].nullable_from_join); // orders — right side stays non-nullable
    }

    // -----------------------------------------------------------------------
    // FULL OUTER JOIN — both sides nullable
    // -----------------------------------------------------------------------
    #[test]
    fn test_build_scope_from_from_full_outer_join() {
        let catalog = make_catalog(&[
            "CREATE TABLE users (id INT NOT NULL);",
            "CREATE TABLE orders (id INT NOT NULL, user_id INT NOT NULL);",
        ]);
        let analyzer = make_analyzer(&catalog);
        let from =
            parse_from("SELECT * FROM users FULL OUTER JOIN orders ON users.id = orders.user_id");
        let scope = analyzer.build_scope_from_from(&from).unwrap();

        assert_eq!(scope.sources.len(), 2);
        assert!(scope.sources[0].nullable_from_join); // both sides nullable
        assert!(scope.sources[1].nullable_from_join);
    }

    // -----------------------------------------------------------------------
    // 7. Subquery in FROM
    // -----------------------------------------------------------------------
    #[test]
    fn test_add_table_factor_subquery() {
        let catalog = make_catalog(&["CREATE TABLE users (id INT NOT NULL, name TEXT NOT NULL);"]);
        let analyzer = make_analyzer(&catalog);
        let from = parse_from("SELECT * FROM (SELECT id, name FROM users) AS sub");
        let scope = analyzer.build_scope_from_from(&from).unwrap();

        assert_eq!(scope.sources.len(), 1);
        assert_eq!(scope.sources[0].alias, "sub");
        assert_eq!(scope.sources[0].columns.len(), 2);
        assert_eq!(scope.sources[0].columns[0].name, "id");
        assert_eq!(scope.sources[0].columns[1].name, "name");
    }

    // -----------------------------------------------------------------------
    // 8. CTE reference in scope
    // -----------------------------------------------------------------------
    #[test]
    fn test_cte_reference_in_build_scope_for_table() {
        let catalog = make_catalog(&[]);
        let mut analyzer = make_analyzer(&catalog);

        // Simulate a CTE being registered
        analyzer.ctes.insert(
            "my_cte".to_string(),
            vec![
                ScopeColumn {
                    name: "x".to_string(),
                    neutral_type: "int32".to_string(),
                    base_nullable: false,
                },
                ScopeColumn {
                    name: "y".to_string(),
                    neutral_type: "string".to_string(),
                    base_nullable: true,
                },
            ],
        );

        let scope = analyzer.build_scope_for_table("my_cte").unwrap();
        assert_eq!(scope.sources.len(), 1);
        assert_eq!(scope.sources[0].alias, "my_cte");
        assert_eq!(scope.sources[0].columns.len(), 2);
        assert_eq!(scope.sources[0].columns[0].name, "x");
        assert_eq!(scope.sources[0].columns[1].name, "y");
    }

    #[test]
    fn test_cte_reference_in_from_clause() {
        let catalog = make_catalog(&[]);
        let mut analyzer = make_analyzer(&catalog);

        analyzer.ctes.insert(
            "my_cte".to_string(),
            vec![ScopeColumn {
                name: "val".to_string(),
                neutral_type: "int32".to_string(),
                base_nullable: false,
            }],
        );

        let from = parse_from("SELECT * FROM my_cte");
        let scope = analyzer.build_scope_from_from(&from).unwrap();
        assert_eq!(scope.sources.len(), 1);
        assert_eq!(scope.sources[0].alias, "my_cte");
        assert_eq!(scope.sources[0].columns[0].name, "val");
    }

    // -----------------------------------------------------------------------
    // 9. get_column_type
    // -----------------------------------------------------------------------
    #[test]
    fn test_get_column_type_found() {
        let catalog = make_catalog(&["CREATE TABLE users (id INT NOT NULL, name TEXT);"]);
        let analyzer = make_analyzer(&catalog);

        let ty = analyzer.get_column_type("users", "id");
        assert!(ty.is_some());
        assert_eq!(ty.unwrap(), "int32");

        let ty = analyzer.get_column_type("users", "name");
        assert!(ty.is_some());
        assert_eq!(ty.unwrap(), "string");
    }

    #[test]
    fn test_get_column_type_not_found() {
        let catalog = make_catalog(&["CREATE TABLE users (id INT NOT NULL);"]);
        let analyzer = make_analyzer(&catalog);

        assert!(analyzer.get_column_type("users", "nonexistent").is_none());
        assert!(analyzer.get_column_type("nonexistent", "id").is_none());
    }

    // -----------------------------------------------------------------------
    // Unknown table in FROM should error
    // -----------------------------------------------------------------------
    #[test]
    fn test_unknown_table_errors() {
        let catalog = make_catalog(&[]);
        let analyzer = make_analyzer(&catalog);
        let from = parse_from("SELECT * FROM nonexistent_table");
        let result = analyzer.build_scope_from_from(&from);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Known set-returning functions
    // -----------------------------------------------------------------------
    #[test]
    fn test_known_function_jsonb_array_elements() {
        let catalog = make_catalog(&[]);
        let analyzer = make_analyzer(&catalog);
        let from = parse_from("SELECT * FROM jsonb_array_elements('[1,2]')");
        let scope = analyzer.build_scope_from_from(&from).unwrap();
        assert_eq!(scope.sources.len(), 1);
        // The function should produce a "value" column
        assert!(scope.sources[0].columns.iter().any(|c| c.name == "value"));
    }
}
