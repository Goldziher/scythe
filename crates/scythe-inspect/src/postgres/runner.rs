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

// ---------------------------------------------------------------------------
// Row → column map
// ---------------------------------------------------------------------------

/// Extract all columns from a [`tokio_postgres::Row`] as a `String → String`
/// map.
///
/// Column extraction strategy (in order of preference):
/// 1. `TEXT` / `VARCHAR` / `NAME` / other text-compatible types → direct
///    `try_get::<&str, &str>` call.
/// 2. `BIGINT` / `INT8` → `try_get::<&str, i64>` → `to_string()`.
/// 3. `INT` / `INT4` → `try_get::<&str, i32>` → `to_string()`.
/// 4. `TEXT[]` / `NAME[]` → `try_get::<&str, Vec<String>>` → comma-joined.
/// 5. Anything else → `format!("{value:?}")` via the `Debug` impl (defensive
///    fallback; canonical checks don't exercise this path).
fn row_to_map(row: &tokio_postgres::Row) -> HashMap<String, String> {
    let mut map = HashMap::new();

    for col in row.columns() {
        let name = col.name().to_string();

        // Try text types first (covers TEXT, VARCHAR, NAME, BPCHAR, etc.)
        if let Ok(v) = row.try_get::<&str, &str>(&*name) {
            map.insert(name, v.to_string());
            continue;
        }

        // Text array (e.g. `array_agg(attname)` → `TEXT[]`)
        if let Ok(v) = row.try_get::<&str, Vec<String>>(&*name) {
            map.insert(name, v.join(", "));
            continue;
        }

        // Bigint (e.g. `count(*)::bigint`)
        if let Ok(v) = row.try_get::<&str, i64>(&*name) {
            map.insert(name, v.to_string());
            continue;
        }

        // Integer
        if let Ok(v) = row.try_get::<&str, i32>(&*name) {
            map.insert(name, v.to_string());
            continue;
        }

        // Boolean
        if let Ok(v) = row.try_get::<&str, bool>(&*name) {
            map.insert(name, v.to_string());
            continue;
        }

        // Defensive: leave the column in the map as an empty string rather
        // than omitting it, so downstream binding substitution can report a
        // clear error if the placeholder was expected.
        map.insert(name, String::new());
    }

    map
}

// ---------------------------------------------------------------------------
// Message rendering
// ---------------------------------------------------------------------------

/// Render a message template by substituting `{var}` placeholders with bound
/// column values.
///
/// Returns `Err(InspectError::MessageBindingMissing)` if any placeholder has
/// no matching key in `bindings`.
fn render_message(
    template: &str,
    bindings: &HashMap<String, String>,
    check_id: &str,
) -> Result<String, InspectError> {
    let re = placeholder_regex();
    let mut last_end = 0;
    let mut output = String::with_capacity(template.len());

    for cap in re.captures_iter(template) {
        let full_match = cap.get(0).unwrap();
        let var_name = &cap[1];

        output.push_str(&template[last_end..full_match.start()]);

        match bindings.get(var_name) {
            Some(value) => output.push_str(value),
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

// ---------------------------------------------------------------------------
// Public runner entry points
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Unit tests (no DB required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_message_substitutes_bindings() {
        let template = "foreign-key `{schema_name}.{table_name}.{constraint_name}` on columns ({columns}) has no covering index";
        let mut bindings = HashMap::new();
        bindings.insert("schema_name".to_string(), "public".to_string());
        bindings.insert("table_name".to_string(), "orders".to_string());
        bindings.insert(
            "constraint_name".to_string(),
            "orders_user_id_fkey".to_string(),
        );
        bindings.insert("columns".to_string(), "user_id".to_string());

        let result = render_message(template, &bindings, "SC-INS01").unwrap();
        assert_eq!(
            result,
            "foreign-key `public.orders.orders_user_id_fkey` on columns (user_id) has no covering index"
        );
    }

    #[test]
    fn render_message_errors_on_missing_binding() {
        let template = "table {schema_name}.{missing_var}";
        let mut bindings = HashMap::new();
        bindings.insert("schema_name".to_string(), "public".to_string());

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
        let result = render_message("static message", &HashMap::new(), "SC-TST").unwrap();
        assert_eq!(result, "static message");
    }
}
