//! Generic check runner — executes a [`CheckSpec`] against a live Postgres
//! connection and converts the result rows into [`Finding`]s.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use scythe_lint::reporters::Finding;
use tokio_postgres::Client;

use crate::error::InspectError;
use crate::spec::CheckSpec;

/// Compiled once on first use; `{var}` placeholder regex shared by every
/// message render call.
fn placeholder_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{(\w+)\}").expect("placeholder regex is valid"))
}

/// One result row, split into the columns that can be rendered into a message
/// and the columns that cannot.
///
/// The split exists because a column this runner cannot decode must not become
/// an empty string. A finding that reads `ratio= ts= f=` looks like it worked
/// and says nothing, and the blanks are indistinguishable from genuinely empty
/// values. Keeping the undecodable columns aside — with the PostgreSQL type
/// name the server reported — lets [`render_message`] fail with something
/// actionable, and only for the checks whose message actually depends on one.
#[derive(Debug, Default)]
pub(crate) struct RowBindings {
    /// `column name → rendered value`, for every column that decoded.
    values: HashMap<String, String>,
    /// `column name → PostgreSQL type name`, for every column that did not.
    unrenderable: HashMap<String, String>,
}

/// Extract every column of a [`tokio_postgres::Row`] that can be rendered as
/// text.
///
/// Decoding is attempted in this order, stopping at the first that succeeds:
/// text-compatible types (`TEXT`/`VARCHAR`/`NAME`/…), `TEXT[]`, `INT8`, `INT4`,
/// `INT2`, `OID`, `FLOAT8`, `FLOAT4`, `BOOL`.
///
/// Anything else — `NUMERIC`, the date/time types, `JSON`, a user type — is
/// recorded as unrenderable rather than blanked. Decoding those needs
/// `tokio-postgres` feature flags (and third-party numeric and date crates)
/// this crate deliberately does not pull in; a check that needs one casts it in
/// its own SQL, which is what the canonical SC-INS13 already does with
/// `round(...)::text`.
fn row_to_map(row: &tokio_postgres::Row) -> RowBindings {
    let mut bindings = RowBindings::default();

    for col in row.columns() {
        let name = col.name().to_string();

        if let Ok(v) = row.try_get::<&str, &str>(&*name) {
            bindings.values.insert(name, v.to_string());
            continue;
        }

        if let Ok(v) = row.try_get::<&str, Vec<String>>(&*name) {
            bindings.values.insert(name, v.join(", "));
            continue;
        }

        if let Ok(v) = row.try_get::<&str, i64>(&*name) {
            bindings.values.insert(name, v.to_string());
            continue;
        }

        if let Ok(v) = row.try_get::<&str, i32>(&*name) {
            bindings.values.insert(name, v.to_string());
            continue;
        }

        if let Ok(v) = row.try_get::<&str, i16>(&*name) {
            bindings.values.insert(name, v.to_string());
            continue;
        }

        if let Ok(v) = row.try_get::<&str, u32>(&*name) {
            bindings.values.insert(name, v.to_string());
            continue;
        }

        if let Ok(v) = row.try_get::<&str, f64>(&*name) {
            bindings.values.insert(name, v.to_string());
            continue;
        }

        if let Ok(v) = row.try_get::<&str, f32>(&*name) {
            bindings.values.insert(name, v.to_string());
            continue;
        }

        if let Ok(v) = row.try_get::<&str, bool>(&*name) {
            bindings.values.insert(name, v.to_string());
            continue;
        }

        let pg_type = col.type_().name().to_string();
        bindings.unrenderable.insert(name, pg_type);
    }

    bindings
}

/// Render a message template by substituting `{var}` placeholders with bound
/// column values.
///
/// # Errors
///
/// - [`InspectError::UnrenderableBinding`] if a placeholder names a column the
///   row carried but this runner could not decode. Reported rather than
///   substituted blank: a message with a hole in it claims to have inspected
///   something it could not read.
/// - [`InspectError::MessageBindingMissing`] if a placeholder names no column
///   at all.
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
                    engine: "postgres",
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

/// Execute `spec.sql` against `client` and return one `(Finding, bindings)`
/// pair per result row.
///
/// The `bindings` map (`column_name → value`) is kept alongside the finding so
/// the caller (e.g. the suppression engine) can match against individual column
/// values without re-parsing the rendered message string.
///
/// # Errors
///
/// - [`InspectError::Query`] if the database rejects the query.
/// - [`InspectError::MessageBindingMissing`] if a `{var}` placeholder in
///   `spec.message` has no matching column in the result set (should be caught
///   at registry-load time by `validate_message_bindings`, but guarded here as
///   a defence-in-depth measure).
/// - [`InspectError::UnrenderableBinding`] if a `{var}` placeholder names a
///   column whose PostgreSQL type this runner cannot decode. Only the check
///   whose message depends on it fails, and it degrades to a warning finding
///   naming the column and the cast that would fix it.
pub async fn run_check_with_bindings(
    client: &Client,
    spec: &CheckSpec,
) -> Result<Vec<(Finding, HashMap<String, String>)>, InspectError> {
    let rows = client
        .query(spec.sql.as_str(), &[])
        .await
        .map_err(|e| InspectError::Query {
            engine: "postgres",
            check_id: spec.id.clone(),
            source: Box::new(e),
        })?;

    let mut pairs = Vec::with_capacity(rows.len());

    for row in rows {
        let bindings = row_to_map(&row);
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

/// Execute `spec.sql` against `client` and return one [`Finding`] per result
/// row.
///
/// This is a thin wrapper around [`run_check_with_bindings`] that drops the
/// bindings after the findings are constructed.  Use it when suppression is not
/// needed (e.g. unit tests, callers that have already applied suppression).
///
/// # Errors
///
/// See [`run_check_with_bindings`].
pub async fn run_check(client: &Client, spec: &CheckSpec) -> Result<Vec<Finding>, InspectError> {
    let pairs = run_check_with_bindings(client, spec).await?;
    Ok(pairs.into_iter().map(|(f, _)| f).collect())
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
        let template =
            "foreign-key `{schema_name}.{table_name}.{constraint_name}` on columns ({columns}) has no covering index";
        let bindings = renderable(&[
            ("schema_name", "public"),
            ("table_name", "orders"),
            ("constraint_name", "orders_user_id_fkey"),
            ("columns", "user_id"),
        ]);

        let result = render_message(template, &bindings, "SC-INS01").unwrap();
        assert_eq!(
            result,
            "foreign-key `public.orders.orders_user_id_fkey` on columns (user_id) has no covering index"
        );
    }

    #[test]
    fn render_message_errors_on_missing_binding() {
        let template = "table {schema_name}.{missing_var}";
        let bindings = renderable(&[("schema_name", "public")]);

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

    /// The reported defect: a `numeric` column rendered as nothing, so
    /// `message = "ratio={ratio}"` emitted `ratio=` and the finding looked like
    /// it had inspected something. It must name the column, its PostgreSQL
    /// type, and the cast that fixes it instead.
    #[test]
    fn should_error_naming_the_column_when_a_placeholder_binds_an_undecodable_type() {
        let bindings = RowBindings {
            values: HashMap::new(),
            unrenderable: HashMap::from([("ratio".to_string(), "numeric".to_string())]),
        };

        let err = render_message("ratio={ratio}", &bindings, "USER-INS-001").unwrap_err();

        let InspectError::UnrenderableBinding {
            check_id,
            binding,
            engine,
            type_name,
        } = &err
        else {
            panic!("expected UnrenderableBinding, got {err:?}");
        };
        assert_eq!(check_id, "USER-INS-001");
        assert_eq!(binding, "ratio");
        assert_eq!(*engine, "postgres");
        assert_eq!(type_name, "numeric");
        assert!(err.to_string().contains("re-alias it as `ratio`"), "{err}");
    }

    /// An undecodable column the message never mentions must not fail the
    /// check: the blast radius is the message that depends on it, nothing more.
    #[test]
    fn should_render_successfully_when_an_undecodable_column_is_not_referenced() {
        let bindings = RowBindings {
            values: HashMap::from([("table_name".to_string(), "orders".to_string())]),
            unrenderable: HashMap::from([("created_at".to_string(), "timestamptz".to_string())]),
        };

        let result = render_message("table {table_name}", &bindings, "USER-INS-002").unwrap();
        assert_eq!(result, "table orders");
    }

    /// An undecodable column and a genuinely absent one are different
    /// problems with different fixes, and the error has to tell them apart.
    #[test]
    fn should_distinguish_an_undecodable_column_from_an_absent_one() {
        let bindings = RowBindings {
            values: HashMap::new(),
            unrenderable: HashMap::from([("ratio".to_string(), "numeric".to_string())]),
        };

        assert!(matches!(
            render_message("{ratio}", &bindings, "SC-TST").unwrap_err(),
            InspectError::UnrenderableBinding { .. }
        ));
        assert!(matches!(
            render_message("{absent}", &bindings, "SC-TST").unwrap_err(),
            InspectError::MessageBindingMissing { .. }
        ));
    }
}
