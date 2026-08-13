use sqlparser::ast::ObjectName;

use crate::dialect::SqlDialect;
use crate::errors::ScytheError;

use super::type_normalizer::{ident_to_lower, normalize_data_type, object_name_to_key, object_name_to_raw_name};
use super::{Catalog, Column, Table};

/// The analyzer's neutral-type vocabulary spellings that never appear as a
/// genuine catalog `sql_type` string (`normalize_data_type` always emits
/// PostgreSQL-flavored spellings: `"integer"`, `"numeric(10,2)"`, `"text"`,
/// never `"int32"`, `"decimal"`, `"string"`). A computed expression column
/// (an aggregate, an arithmetic expression, ...) whose type does not trace
/// back to a single real source column falls back to its neutral type name
/// (see `AnalyzedColumn::from_type_info`), so it must be translated back
/// into a real SQL type label before it is stored as a `Column::sql_type`
/// -- otherwise a view built on `SUM(numeric_col)` would register a column
/// typed the literal string `"decimal"`. Mirrors
/// `analyzer::helpers::neutral_to_sql_label`, which is `pub(super)` to the
/// analyzer module and not reachable from here. See issue #182.
fn neutral_label_to_sql_type(label: &str) -> &str {
    match label {
        "int16" => "smallint",
        "int32" => "integer",
        "int64" => "bigint",
        "float32" => "real",
        "float64" => "double precision",
        "decimal" => "numeric",
        "string" => "text",
        "bool" => "boolean",
        "bytes" => "bytea",
        "time_tz" => "timetz",
        "datetime" => "timestamp",
        "datetime_tz" => "timestamptz",
        other => other,
    }
}

impl Catalog {
    pub(super) fn process_create_view(
        &mut self,
        name: ObjectName,
        view_columns: Vec<sqlparser::ast::ViewColumnDef>,
        query: sqlparser::ast::Query,
        _materialized: bool,
        dialect: &SqlDialect,
    ) -> Result<(), ScytheError> {
        let view_key = object_name_to_key(&name);
        let raw_name = object_name_to_raw_name(&name);

        if !view_columns.is_empty() {
            let columns: Vec<Column> = view_columns
                .iter()
                .map(|vc| {
                    let sql_type = vc
                        .data_type
                        .as_ref()
                        .map(|dt| {
                            let (t, _) = normalize_data_type(dt, &self.domains, *dialect);
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
            self.tables.insert(view_key, Table { columns, raw_name });
            return Ok(());
        }

        let columns = self.resolve_select_columns(query)?;
        self.tables.insert(view_key, Table { columns, raw_name });
        Ok(())
    }

    /// Resolve a raw `SELECT` query's projected output columns by
    /// delegating to the query analyzer -- the exact same code path used to
    /// resolve an ordinary annotated `-- name: Foo :many` query -- rather
    /// than re-deriving column metadata (join nullability, aggregate return
    /// types, `UNION` shapes, arbitrary expressions) with a second,
    /// disagreeing implementation. Shared by view resolution (issue #182)
    /// and `CREATE TABLE ... AS SELECT` (issue #183).
    pub(super) fn resolve_select_columns(&self, query: sqlparser::ast::Query) -> Result<Vec<Column>, ScytheError> {
        let wrapped = crate::parser::Query {
            name: String::new(),
            command: crate::parser::QueryCommand::Many,
            sql: String::new(),
            stmt: sqlparser::ast::Statement::Query(Box::new(query)),
            annotations: crate::parser::Annotations::default(),
        };

        let analyzed = crate::analyzer::analyze(self, &wrapped)?;

        Ok(analyzed
            .columns
            .into_iter()
            .map(|c| Column {
                name: c.name,
                sql_type: neutral_label_to_sql_type(&c.sql_type).to_string(),
                nullable: c.nullable,
                default: None,
                primary_key: false,
            })
            .collect())
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
        let view = catalog.get_table("active_users").expect("view should exist");
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
        let view = catalog.get_table("mv").expect("materialized view should exist");
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
        let view = catalog.get_table("active_names").expect("view-from-view should exist");
        assert_eq!(view.columns.len(), 1);
        assert_eq!(view.columns[0].name, "name");
        assert_eq!(view.columns[0].sql_type, "text");
    }

    // -- #182: view resolver correctness --------------------------------

    #[test]
    fn test_left_join_widens_nullability_through_view() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL);",
            "CREATE TABLE profiles (id SERIAL PRIMARY KEY, user_id INTEGER NOT NULL, bio TEXT NOT NULL);",
            "CREATE VIEW v AS SELECT u.id, p.bio FROM users u LEFT JOIN profiles p ON u.id = p.user_id;",
        ])
        .unwrap();
        let view = catalog.get_table("v").unwrap();
        let bio = view.columns.iter().find(|c| c.name == "bio").unwrap();
        assert!(
            bio.nullable,
            "a column from the outer side of a LEFT JOIN must be nullable through a view, exactly as it is in a direct query"
        );
    }

    #[test]
    fn test_view_aggregate_type_is_not_hardcoded() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE sales (amount NUMERIC(10,2) NOT NULL, weight DOUBLE PRECISION NOT NULL);",
            "CREATE VIEW v AS SELECT SUM(amount) AS total, SUM(weight) AS wtotal FROM sales;",
        ])
        .unwrap();
        let view = catalog.get_table("v").unwrap();
        let total = view.columns.iter().find(|c| c.name == "total").unwrap();
        let wtotal = view.columns.iter().find(|c| c.name == "wtotal").unwrap();
        assert_eq!(
            total.sql_type, "numeric",
            "SUM(numeric) must resolve to numeric, not the previously-hardcoded bigint"
        );
        assert_eq!(
            wtotal.sql_type, "double precision",
            "SUM(double precision) must resolve to double precision, not the previously-hardcoded bigint"
        );
    }

    #[test]
    fn test_union_view_resolves_columns() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE a (id INTEGER NOT NULL, v TEXT NOT NULL);",
            "CREATE TABLE b (id INTEGER NOT NULL, v TEXT NOT NULL);",
            "CREATE VIEW vu AS SELECT id, v FROM a UNION ALL SELECT id, v FROM b;",
        ])
        .unwrap();
        let view = catalog.get_table("vu").expect("UNION view must be registered");
        assert_eq!(
            view.columns.len(),
            2,
            "a UNION view must resolve its output columns, not zero"
        );
        assert_eq!(view.columns[0].name, "id");
        assert_eq!(view.columns[1].name, "v");
    }

    #[test]
    fn test_view_expression_columns_are_not_named_unknown() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE a (id INTEGER NOT NULL, v TEXT NOT NULL);",
            "CREATE VIEW ve AS SELECT id + 1, upper(v) FROM a;",
        ])
        .unwrap();
        let view = catalog
            .get_table("ve")
            .expect("view with expression columns must resolve");
        let names: Vec<&String> = view.columns.iter().map(|c| &c.name).collect();
        assert!(
            view.columns.iter().all(|c| c.name != "unknown"),
            "expression columns must get a real name through the analyzer, not the literal placeholder \"unknown\": {names:?}"
        );
    }
}
