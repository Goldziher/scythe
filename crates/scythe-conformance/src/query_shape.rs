//! A minimal, best-effort model of a query's output shape: whether it has a
//! top-level `ORDER BY`, and -- for each SELECT-list item that is a direct
//! column reference -- the physical table/column it projects.
//!
//! Parsed once via [`sqlparser`] and reused by two independent fixture
//! checks: the ORDER BY determinism check
//! (`fixture::FixtureError::MissingOrderBy`) and the analyzer/live-schema
//! reconciliation check
//! (`fixture::FixtureError::LiveSchemaColumnMissing`). Both used to be a
//! substring search over the raw SQL text; parsing the AST instead means a
//! `--` comment, a string literal, or a `ROW_NUMBER() OVER (ORDER BY id)`
//! window function can no longer be mistaken for a real top-level
//! `ORDER BY`, and aliased output columns (`o.created_at AS
//! order_created_at`) can be traced back to the physical column they came
//! from instead of only being checked by name.
//!
//! This is deliberately *not* a general SQL semantic resolver: subqueries,
//! `UNION`s, wildcards, and computed expressions are left unresolved
//! (`source_column` returns `None` for them) rather than guessed at. A
//! false "column missing" on an expression this walk can't understand would
//! be worse than not checking it at all.

use ahash::AHashMap;
use sqlparser::ast::{Expr, SelectItem, SetExpr, Statement, TableFactor};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::{Parser, ParserError};

/// A direct column reference found in a query's SELECT list: `column`,
/// optionally qualified by the table alias (or bare table name) it was
/// written against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnRef {
    pub qualifier: Option<String>,
    pub column: String,
}

/// The parsed shape of a query, as far as this best-effort walk can resolve
/// it.
#[derive(Debug, Clone, Default)]
pub struct QueryShape {
    /// Whether the outermost `Query` node has a top-level `ORDER BY`.
    pub has_order_by: bool,
    /// (output column name, physical source) for each SELECT-list item that
    /// is a direct column reference. `None` for anything else (computed
    /// expressions, function calls, `*`).
    projections: Vec<(String, Option<ColumnRef>)>,
    /// Table alias (or bare table name, when unaliased) -> physical table
    /// name, collected from `FROM` and `JOIN` clauses.
    aliases: AHashMap<String, String>,
}

impl QueryShape {
    /// The physical source column for `output_name`, if the SELECT list
    /// projects it as a direct (possibly qualified) column reference.
    pub fn source_column(&self, output_name: &str) -> Option<&ColumnRef> {
        self.projections
            .iter()
            .find(|(name, _)| name == output_name)
            .and_then(|(_, source)| source.as_ref())
    }
}

/// Parse `sql` (a fixture's `query_sql`, including any leading `-- @name`
/// annotation comments -- `--` is a valid SQL line comment, so the
/// tokenizer skips them) into a [`QueryShape`].
pub fn parse(sql: &str) -> Result<QueryShape, ParserError> {
    let statements = Parser::parse_sql(&GenericDialect {}, sql)?;

    let mut shape = QueryShape::default();

    for stmt in &statements {
        let Statement::Query(query) = stmt else { continue };
        shape.has_order_by |= query.order_by.is_some();

        let SetExpr::Select(select) = query.body.as_ref() else {
            continue;
        };

        for table_with_joins in &select.from {
            collect_table_alias(&table_with_joins.relation, &mut shape.aliases);
            for join in &table_with_joins.joins {
                collect_table_alias(&join.relation, &mut shape.aliases);
            }
        }

        for item in &select.projection {
            match item {
                SelectItem::UnnamedExpr(expr) => {
                    if let Some(name) = expr_output_name(expr) {
                        shape.projections.push((name, source_column(expr)));
                    }
                }
                SelectItem::ExprWithAlias { expr, alias } => {
                    shape.projections.push((alias.value.clone(), source_column(expr)));
                }
                SelectItem::ExprWithAliases { .. } | SelectItem::QualifiedWildcard(..) | SelectItem::Wildcard(_) => {}
            }
        }
    }

    Ok(shape)
}

/// Whether `column` (a physical column name from [`ColumnRef`]) exists in
/// `catalog`: if `source.qualifier` resolves to a known table alias, only
/// that table is checked; otherwise every table `shape` knows about is
/// checked, since an unqualified reference in a single-table query (or one
/// this walk couldn't disambiguate) can't be pinned to one table.
pub fn column_exists(catalog: &scythe_core::catalog::Catalog, shape: &QueryShape, source: &ColumnRef) -> bool {
    let has_column = |table_name: &str| {
        catalog.get_table(table_name).is_some_and(|table| {
            table
                .columns
                .iter()
                .any(|c| c.name.eq_ignore_ascii_case(&source.column))
        })
    };

    if let Some(qualifier) = &source.qualifier
        && let Some(table_name) = shape.aliases.get(qualifier)
    {
        return has_column(table_name);
    }

    shape.aliases.values().any(|table_name| has_column(table_name))
}

fn collect_table_alias(factor: &TableFactor, aliases: &mut AHashMap<String, String>) {
    if let TableFactor::Table { name, alias, .. } = factor {
        let table_name = name.to_string();
        let key = alias
            .as_ref()
            .map(|a| a.name.value.clone())
            .unwrap_or_else(|| table_name.clone());
        aliases.insert(key, table_name);
    }
}

fn source_column(expr: &Expr) -> Option<ColumnRef> {
    match expr {
        Expr::Identifier(ident) => Some(ColumnRef {
            qualifier: None,
            column: ident.value.clone(),
        }),
        Expr::CompoundIdentifier(parts) => {
            let column = parts.last()?.value.clone();
            let qualifier = (parts.len() >= 2).then(|| parts[parts.len() - 2].value.clone());
            Some(ColumnRef { qualifier, column })
        }
        _ => None,
    }
}

fn expr_output_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Identifier(ident) => Some(ident.value.clone()),
        Expr::CompoundIdentifier(parts) => parts.last().map(|p| p.value.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_top_level_order_by() {
        let shape = parse("SELECT id FROM t ORDER BY id").unwrap();
        assert!(shape.has_order_by);
    }

    #[test]
    fn does_not_mistake_a_comment_for_an_order_by() {
        let shape = parse("-- ORDER BY is required\nSELECT id FROM t").unwrap();
        assert!(!shape.has_order_by);
    }

    #[test]
    fn does_not_mistake_a_string_literal_for_an_order_by() {
        let shape = parse("SELECT id FROM t WHERE name = 'ORDER BY nonsense'").unwrap();
        assert!(!shape.has_order_by);
    }

    #[test]
    fn does_not_mistake_a_window_function_order_by_for_a_top_level_one() {
        let shape = parse("SELECT id, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t").unwrap();
        assert!(!shape.has_order_by);
    }

    #[test]
    fn detects_an_order_by_split_across_lines() {
        let shape = parse("SELECT id FROM t ORDER\n  BY id").unwrap();
        assert!(shape.has_order_by);
    }

    #[test]
    fn resolves_a_qualified_column_reference_through_an_alias() {
        let shape = parse("SELECT o.created_at AS order_created_at FROM orders o").unwrap();
        assert_eq!(
            shape.source_column("order_created_at"),
            Some(&ColumnRef {
                qualifier: Some("o".to_string()),
                column: "created_at".to_string(),
            })
        );
    }

    #[test]
    fn resolves_an_unqualified_column_reference() {
        let shape = parse("SELECT id FROM t").unwrap();
        assert_eq!(
            shape.source_column("id"),
            Some(&ColumnRef {
                qualifier: None,
                column: "id".to_string(),
            })
        );
    }

    #[test]
    fn returns_none_for_a_computed_expression() {
        let shape = parse("SELECT COUNT(*) AS total FROM t").unwrap();
        assert_eq!(shape.source_column("total"), None);
    }

    #[test]
    fn returns_none_for_an_unknown_output_name() {
        let shape = parse("SELECT id FROM t").unwrap();
        assert_eq!(shape.source_column("nonexistent"), None);
    }

    #[test]
    fn parse_errors_on_invalid_sql() {
        assert!(parse("SELECT FROM WHERE ORDER BY").is_err());
    }
}
