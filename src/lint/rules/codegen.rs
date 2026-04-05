use sqlparser::ast::*;

use crate::lint::rule::LintRule;
use crate::lint::types::*;
use crate::parser::QueryCommand;

// ---------------------------------------------------------------------------
// SC-C01: MissingReturnsAnnotation
// ---------------------------------------------------------------------------

/// No-op rule — the parser already enforces @returns. Kept as a placeholder
/// so the rule ID space is consistent.
pub struct MissingReturnsAnnotation;

impl LintRule for MissingReturnsAnnotation {
    fn id(&self) -> &'static str {
        "SC-C01"
    }
    fn name(&self) -> &'static str {
        "missing-returns-annotation"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Codegen
    }
    fn default_severity(&self) -> Severity {
        Severity::Off
    }
    fn description(&self) -> &'static str {
        "Query should have a @returns annotation (already enforced by parser)"
    }
}

// ---------------------------------------------------------------------------
// SC-C02: ExecWithReturning
// ---------------------------------------------------------------------------

pub struct ExecWithReturning;

impl LintRule for ExecWithReturning {
    fn id(&self) -> &'static str {
        "SC-C02"
    }
    fn name(&self) -> &'static str {
        "exec-with-returning"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Codegen
    }
    fn default_severity(&self) -> Severity {
        Severity::Warn
    }
    fn description(&self) -> &'static str {
        ":exec command but query has RETURNING clause — returned rows will be discarded"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        let is_exec = matches!(ctx.analyzed.command, QueryCommand::Exec);
        if !is_exec {
            return Vec::new();
        }

        let has_returning = match ctx.stmt {
            Statement::Insert(ins) => ins.returning.is_some(),
            Statement::Update(upd) => upd.returning.is_some(),
            Statement::Delete(del) => del.returning.is_some(),
            _ => false,
        };

        if has_returning {
            return vec![Violation {
                rule_id: self.id(),
                message: ":exec command with RETURNING clause — returned rows will be discarded"
                    .into(),
                fix: Some(LintFix {
                    description: "Change to :one or :many, or remove RETURNING".into(),
                    replacement: String::new(),
                }),
            }];
        }
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// SC-C03: DuplicateQueryNames
// ---------------------------------------------------------------------------

/// This rule is mostly handled at the engine level (build_report detects dups).
/// The rule struct exists so it shows up in the registry but its check_query
/// is a no-op — the engine does the cross-query dedup.
pub struct DuplicateQueryNames;

impl LintRule for DuplicateQueryNames {
    fn id(&self) -> &'static str {
        "SC-C03"
    }
    fn name(&self) -> &'static str {
        "duplicate-query-names"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Codegen
    }
    fn default_severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &'static str {
        "Multiple queries share the same @name"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer;
    use crate::catalog::Catalog;
    use crate::lint::rule::LintRule;
    use crate::parser::parse_query;

    fn make_catalog() -> Catalog {
        Catalog::from_ddl(&["CREATE TABLE users (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                email TEXT NOT NULL
            );"])
        .unwrap()
    }

    fn make_ctx<'a>(
        query: &'a crate::parser::Query,
        analyzed: &'a crate::analyzer::AnalyzedQuery,
        catalog: &'a Catalog,
    ) -> LintContext<'a> {
        LintContext {
            sql: &query.sql,
            stmt: &query.stmt,
            analyzed,
            catalog,
            annotations: &query.annotations,
        }
    }

    // SC-C02

    #[test]
    fn exec_with_returning_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name CreateUser\n-- @returns :exec\nINSERT INTO users (name, email) VALUES ($1, $2) RETURNING id;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = ExecWithReturning.check_query(&ctx);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule_id, "SC-C02");
    }

    #[test]
    fn exec_without_returning_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name CreateUser\n-- @returns :exec\nINSERT INTO users (name, email) VALUES ($1, $2);",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = ExecWithReturning.check_query(&ctx);
        assert!(v.is_empty());
    }

    #[test]
    fn one_with_returning_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name CreateUser\n-- @returns :one\nINSERT INTO users (name, email) VALUES ($1, $2) RETURNING id;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = ExecWithReturning.check_query(&ctx);
        assert!(v.is_empty());
    }
}
