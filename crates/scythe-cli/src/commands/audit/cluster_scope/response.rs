//! Strict reader for the parser-helper JSON envelope.
//!
//! Exit 0 from a helper is **not** evidence of a usable AST: a malformed or
//! truncated envelope must fail closed. Every field the policy later reads is
//! validated here first, so the classifier only ever sees an AST whose parser
//! release and PostgreSQL major version match the requested one and whose
//! statement list is non-empty and structurally complete.

use serde::Deserialize;
use serde_json::Value;

use super::helper::PgVersion;

/// Which grammar the helper was asked to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseMode {
    Sql,
    Plpgsql,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    protocol: u64,
    pg_major: u64,
    pg_version: String,
    parser_release: String,
    ast: Value,
}

/// A parse tree whose envelope has been fully validated.
pub struct ValidatedAst {
    ast: Value,
}

impl ValidatedAst {
    /// Validate the raw helper response for `version` and `mode`.
    pub fn parse(bytes: &[u8], version: PgVersion, mode: ParseMode) -> Result<Self, String> {
        let response: Response =
            serde_json::from_slice(bytes).map_err(|error| format!("parser helper returned invalid JSON: {error}"))?;
        let (expected_major, expected_pg, expected_release) = version.expected();
        if response.protocol != 1
            || response.pg_major != expected_major
            || response.pg_version != expected_pg
            || response.parser_release != expected_release
        {
            return Err(format!(
                "parser helper returned protocol {} / PostgreSQL {} ({}) but {} was requested",
                response.protocol, response.pg_version, response.parser_release, expected_pg
            ));
        }

        match mode {
            ParseMode::Sql => {
                let Some(stmts) = response.ast.get("stmts").and_then(Value::as_array) else {
                    return Err("parser helper returned no SQL statement list".to_string());
                };
                if stmts.is_empty()
                    || stmts.iter().any(|entry| {
                        entry
                            .get("stmt")
                            .and_then(Value::as_object)
                            .is_none_or(|statement| statement.len() != 1)
                    })
                {
                    return Err("parser helper returned an empty or incomplete SQL AST".to_string());
                }
            }
            ParseMode::Plpgsql => {
                let Some(functions) = response.ast.as_array() else {
                    return Err("parser helper returned no PL/pgSQL function list".to_string());
                };
                if functions.is_empty()
                    || functions.iter().any(|entry| {
                        entry
                            .get("PLpgSQL_function")
                            .and_then(Value::as_object)
                            .and_then(|function| function.get("action"))
                            .and_then(Value::as_object)
                            .is_none()
                    })
                {
                    return Err("parser helper returned an empty or incomplete PL/pgSQL AST".to_string());
                }
            }
        }

        Ok(Self { ast: response.ast })
    }

    pub fn as_value(&self) -> &Value {
        &self.ast
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sql_response(ast: Value) -> String {
        serde_json::json!({
            "protocol": 1, "pg_major": 15, "pg_version": "15.1", "parser_release": "15-4.2.4",
            "ast": ast
        })
        .to_string()
    }

    #[test]
    fn rejects_empty_sql_tree_before_policy_can_read_it() {
        let response = sql_response(serde_json::json!({"version": 150001, "stmts": []}));
        assert!(ValidatedAst::parse(response.as_bytes(), PgVersion::Pg15, ParseMode::Sql).is_err());
    }

    #[test]
    fn rejects_wrong_parser_version_and_wrong_ast_mode() {
        let response = sql_response(serde_json::json!({"stmts": [{"stmt": {"CreateRoleStmt": {}}}]}));
        assert!(ValidatedAst::parse(response.as_bytes(), PgVersion::Pg18, ParseMode::Sql).is_err());
        assert!(ValidatedAst::parse(response.as_bytes(), PgVersion::Pg15, ParseMode::Plpgsql).is_err());
    }

    #[test]
    fn rejects_incomplete_plpgsql_function() {
        let response = serde_json::json!({
            "protocol": 1, "pg_major": 18, "pg_version": "18.6", "parser_release": "18.1.0",
            "ast": [{"PLpgSQL_function": {"datums": []}}]
        })
        .to_string();
        assert!(ValidatedAst::parse(response.as_bytes(), PgVersion::Pg18, ParseMode::Plpgsql).is_err());
    }

    #[test]
    fn admits_validated_sql_ast() {
        let response = sql_response(serde_json::json!({"stmts": [{"stmt": {"CreateRoleStmt": {}}}]}));
        let ast =
            ValidatedAst::parse(response.as_bytes(), PgVersion::Pg15, ParseMode::Sql).expect("valid AST envelope");
        assert_eq!(
            ast.as_value()["stmts"][0]["stmt"]["CreateRoleStmt"],
            serde_json::json!({})
        );
    }

    #[test]
    fn rejects_a_response_with_unknown_fields() {
        let response = serde_json::json!({
            "protocol": 1, "pg_major": 15, "pg_version": "15.1", "parser_release": "15-4.2.4",
            "ast": {"stmts": [{"stmt": {"CreateStmt": {}}}]}, "extra": true
        })
        .to_string();
        assert!(ValidatedAst::parse(response.as_bytes(), PgVersion::Pg15, ParseMode::Sql).is_err());
    }
}
