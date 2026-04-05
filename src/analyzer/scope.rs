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
}
