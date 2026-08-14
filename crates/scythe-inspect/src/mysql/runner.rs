//! Generic check runner — executes a [`CheckSpec`] against a live
//! MySQL/MariaDB connection and converts the result rows into [`Finding`]s.
//!
//! Mirrors `postgres::runner`'s shape (decode-or-report per column, then
//! render the message template from the decoded columns); see that module's
//! docs for the rationale a column that cannot be decoded must be *reported*,
//! not rendered as an empty string.

use std::collections::HashMap;
use std::sync::OnceLock;

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Row, Value};
use regex::Regex;
use scythe_lint::reporters::Finding;

use crate::error::InspectError;
use crate::spec::CheckSpec;

/// Compiled once on first use; `{var}` placeholder regex shared by every
/// message render call.
fn placeholder_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{(\w+)\}").expect("placeholder regex is valid"))
}

/// One result row, split into the columns that can be rendered into a message
/// and the columns that cannot. See `postgres::runner::RowBindings` for why
/// the split exists — the same reasoning applies here unchanged.
#[derive(Debug, Default)]
pub(crate) struct RowBindings {
    /// `column name → rendered value`, for every column that decoded.
    values: HashMap<String, String>,
    /// `column name → MySQL wire type name`, for every column that did not.
    unrenderable: HashMap<String, String>,
}

/// Extract every column of a `mysql_async::Row` that can be rendered as text.
///
/// `Value::Bytes` covers MySQL's text-protocol strings *and* `DECIMAL`/
/// `NEWDECIMAL` — unlike PostgreSQL's `numeric`, MySQL returns exact-numeric
/// columns as their textual representation over the wire, so no separate
/// numeric decode path (and no extra dependency) is needed to render one.
/// `NULL` renders as an empty string: it is a genuinely absent value, not one
/// this runner failed to decode, and belongs in the same bucket as an empty
/// PostgreSQL text column.
///
/// `Value::Date` and `Value::Time` are recorded as unrenderable rather than
/// hand-formatted: no canonical check in this crate projects one into a
/// message today, and hand-rolling a date format here risks disagreeing with
/// whatever format the column's message placeholder actually needs.
fn row_to_map(row: &Row) -> RowBindings {
    let mut bindings = RowBindings::default();

    for (index, column) in row.columns_ref().iter().enumerate() {
        let name = column.name_str().into_owned();

        match row.as_ref(index) {
            Some(Value::Bytes(bytes)) => {
                bindings
                    .values
                    .insert(name, String::from_utf8_lossy(bytes).into_owned());
            }
            Some(Value::Int(v)) => {
                bindings.values.insert(name, v.to_string());
            }
            Some(Value::UInt(v)) => {
                bindings.values.insert(name, v.to_string());
            }
            Some(Value::Float(v)) => {
                bindings.values.insert(name, v.to_string());
            }
            Some(Value::Double(v)) => {
                bindings.values.insert(name, v.to_string());
            }
            Some(Value::NULL) => {
                bindings.values.insert(name, String::new());
            }
            Some(Value::Date(..)) | Some(Value::Time(..)) => {
                bindings
                    .unrenderable
                    .insert(name, format!("{:?}", column.column_type()));
            }
            None => {
                bindings.unrenderable.insert(name, "unknown".to_string());
            }
        }
    }

    bindings
}

/// Render a message template by substituting `{var}` placeholders with bound
/// column values. Identical contract to `postgres::runner::render_message`.
fn render_message(template: &str, bindings: &RowBindings, check_id: &str) -> Result<String, InspectError> {
    let re = placeholder_regex();
    let mut last_end = 0;
    let mut output = String::with_capacity(template.len());

    for cap in re.captures_iter(template) {
        let full_match = cap.get(0).unwrap();
        let var_name = &cap[1];

        output.push_str(&template[last_end..full_match.start()]);

        match bindings.values.get(var_name) {
            Some(value) => output.push_str(value),
            None if bindings.unrenderable.contains_key(var_name) => {
                return Err(InspectError::UnrenderableBinding {
                    check_id: check_id.to_string(),
                    binding: var_name.to_string(),
                    engine: "mysql",
                    type_name: bindings.unrenderable[var_name].clone(),
                });
            }
            None => {
                return Err(InspectError::MessageBindingMissing {
                    check_id: check_id.to_string(),
                    binding: var_name.to_string(),
                });
            }
        }

        last_end = full_match.end();
    }

    output.push_str(&template[last_end..]);
    Ok(output)
}

/// Execute `spec.sql` against `conn` and return one `(Finding, bindings)`
/// pair per result row.
///
/// `conn` is `&mut` — unlike `tokio_postgres::Client`, `mysql_async::Conn` is
/// an exclusively-owned stream and every query needs mutable access to it.
///
/// # Errors
///
/// - [`InspectError::Query`] if the database rejects the query.
/// - [`InspectError::MessageBindingMissing`] if a `{var}` placeholder in
///   `spec.message` has no matching column in the result set (should be caught
///   at registry-load time by `validate_message_bindings`, but guarded here as
///   a defence-in-depth measure).
/// - [`InspectError::UnrenderableBinding`] if a `{var}` placeholder names a
///   column whose MySQL wire type this runner cannot decode.
pub async fn run_check_with_bindings(
    conn: &mut Conn,
    spec: &CheckSpec,
) -> Result<Vec<(Finding, HashMap<String, String>)>, InspectError> {
    let rows: Vec<Row> = conn.query(spec.sql.as_str()).await.map_err(|e| InspectError::Query {
        engine: "mysql",
        check_id: spec.id.clone(),
        source: Box::new(e),
    })?;

    let mut pairs = Vec::with_capacity(rows.len());

    for row in &rows {
        let bindings = row_to_map(row);
        let message = render_message(&spec.message, &bindings, &spec.id)?;
        let bindings = bindings.values;

        let finding = Finding {
            file: String::new(),
            query_name: None,
            rule_id: spec.id.clone(),
            rule_name: Some(spec.name.clone()),
            rule_description: Some(spec.description.clone()),
            severity: spec.severity,
            message,
            line: None,
            column: None,
            cwe: spec.cwe.clone(),
            source: Some("inspect".to_string()),
        };

        pairs.push((finding, bindings));
    }

    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderable(pairs: &[(&str, &str)]) -> RowBindings {
        RowBindings {
            values: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            unrenderable: HashMap::new(),
        }
    }

    #[test]
    fn render_message_substitutes_bindings() {
        let template = "`{schema_name}.{table_name}` has no PRIMARY KEY";
        let bindings = renderable(&[("schema_name", "app"), ("table_name", "orders")]);

        let result = render_message(template, &bindings, "SC-INS-MY01").unwrap();
        assert_eq!(result, "`app.orders` has no PRIMARY KEY");
    }

    #[test]
    fn render_message_errors_on_missing_binding() {
        let template = "table {schema_name}.{missing_var}";
        let bindings = renderable(&[("schema_name", "app")]);

        let err = render_message(template, &bindings, "SC-TST").unwrap_err();
        match err {
            InspectError::MessageBindingMissing { check_id, binding } => {
                assert_eq!(check_id, "SC-TST");
                assert_eq!(binding, "missing_var");
            }
            other => panic!("expected MessageBindingMissing, got {other:?}"),
        }
    }

    #[test]
    fn render_message_handles_no_placeholders() {
        let result = render_message("static message", &RowBindings::default(), "SC-TST").unwrap();
        assert_eq!(result, "static message");
    }

    /// A column this runner could not decode must not silently blank out —
    /// the error must name the column, the MySQL type, and the engine, so a
    /// PostgreSQL-worded message never leaks into a MySQL check's diagnostic.
    #[test]
    fn should_error_naming_the_column_and_engine_when_a_placeholder_binds_an_undecodable_type() {
        let bindings = RowBindings {
            values: HashMap::new(),
            unrenderable: HashMap::from([("seen_at".to_string(), "MYSQL_TYPE_DATE".to_string())]),
        };

        let err = render_message("seen_at={seen_at}", &bindings, "USER-INS-MYSQL-001").unwrap_err();

        let InspectError::UnrenderableBinding {
            check_id,
            binding,
            engine,
            type_name,
        } = &err
        else {
            panic!("expected UnrenderableBinding, got {err:?}");
        };
        assert_eq!(check_id, "USER-INS-MYSQL-001");
        assert_eq!(binding, "seen_at");
        assert_eq!(*engine, "mysql");
        assert_eq!(type_name, "MYSQL_TYPE_DATE");
    }

    /// An undecodable column the message never mentions must not fail the
    /// check: the blast radius is the message that depends on it, nothing more.
    #[test]
    fn should_render_successfully_when_an_undecodable_column_is_not_referenced() {
        let bindings = RowBindings {
            values: HashMap::from([("table_name".to_string(), "orders".to_string())]),
            unrenderable: HashMap::from([("seen_at".to_string(), "MYSQL_TYPE_DATE".to_string())]),
        };

        let result = render_message("table {table_name}", &bindings, "USER-INS-MYSQL-002").unwrap();
        assert_eq!(result, "table orders");
    }

    /// A NULL column is a legitimate empty value, not an undecodable one —
    /// it must render as an empty string rather than fail the check.
    #[test]
    fn should_render_a_null_column_as_empty_string() {
        let bindings = RowBindings {
            values: HashMap::from([("comment".to_string(), String::new())]),
            unrenderable: HashMap::new(),
        };

        let result = render_message("comment=[{comment}]", &bindings, "USER-INS-MYSQL-003").unwrap();
        assert_eq!(result, "comment=[]");
    }
}
