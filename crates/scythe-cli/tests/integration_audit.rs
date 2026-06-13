//! Hand-written integration tests for suppression and user rules.
//!
//! These are distinct from `tests/generated/` (gitignored, fixture-driven) and
//! cover the suppression / user-rule paths that the fixture harness cannot reach.
//!
//! Because scythe-cli is a `[[bin]]` crate with no `[lib]` target we cannot
//! import its internal functions directly.  The test helper below re-implements
//! the relevant logic using the public scythe-lint API so the tests remain
//! self-contained.

use std::io::Write;

use scythe_core::analyzer::AnalyzedQuery;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::Annotations;
use scythe_lint::reporters::Finding;
use scythe_lint::types::RuleCategory;
use scythe_lint::{
    AuditConfigError, LintContext, MatcherRegistry, RuleRegistry, RuleSpec, Severity,
    SuppressionSet, default_registry, extract_cwe, load_rules_from_file, register_user_rules,
};

// ---------------------------------------------------------------------------
// Shared helper: run security rules over raw SQL, respecting suppressions.
// ---------------------------------------------------------------------------

fn run_rules(
    path: &str,
    sql: &str,
    dialect: &SqlDialect,
    catalog: &Catalog,
    registry: &RuleRegistry,
) -> Vec<Finding> {
    use sqlparser::tokenizer::{Token, Tokenizer};

    let rules = registry.active_rules();
    let suppressions = SuppressionSet::parse(sql);

    let parser_dialect = dialect.to_sqlparser_dialect();
    let statements = match sqlparser::parser::Parser::parse_sql(parser_dialect.as_ref(), sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let n = statements.len();
    let mut start_lines = vec![1usize; n];
    if let Ok(tokens) = Tokenizer::new(parser_dialect.as_ref(), sql).tokenize_with_location() {
        let mut idx = 0usize;
        let mut recorded = false;
        for t in &tokens {
            let line = t.span.start.line as usize;
            match &t.token {
                Token::Whitespace(_) => continue,
                Token::SemiColon => {
                    idx += 1;
                    recorded = false;
                    if idx >= n {
                        break;
                    }
                    continue;
                }
                _ => {
                    if !recorded {
                        start_lines[idx] = line;
                        recorded = true;
                    }
                }
            }
        }
    }

    let empty_annotations = Annotations::default();
    let empty_analyzed = AnalyzedQuery::default();
    let mut findings = Vec::new();

    for (stmt_idx, stmt) in statements.iter().enumerate() {
        let stmt_line = start_lines[stmt_idx];
        let ctx = LintContext {
            sql,
            stmt,
            analyzed: &empty_analyzed,
            catalog,
            annotations: &empty_annotations,
            dialect: *dialect,
        };
        for (rule, severity) in &rules {
            if !matches!(rule.category(), RuleCategory::Security) {
                continue;
            }
            for violation in rule.check_query(&ctx) {
                if !suppressions.is_empty()
                    && suppressions.is_suppressed(&violation.rule_id, stmt_line)
                {
                    continue;
                }
                findings.push(Finding {
                    file: path.to_string(),
                    query_name: None,
                    rule_id: violation.rule_id.to_string(),
                    rule_name: Some(rule.name().to_string()),
                    rule_description: Some(rule.description().to_string()),
                    severity: *severity,
                    message: violation.message,
                    line: None,
                    column: None,
                    cwe: extract_cwe(rule.description()),
                });
            }
        }
    }

    findings
}

// ---------------------------------------------------------------------------
// Test 1: suppression — one suppressed, one unsuppressed GRANT ALL
// ---------------------------------------------------------------------------

#[test]
fn suppression_drops_annotated_grant_and_keeps_plain_grant() {
    let dialect = SqlDialect::PostgreSQL;
    let catalog = Catalog::from_ddl_with_dialect(&[], &dialect).expect("empty catalog");
    let registry = default_registry();

    // The first GRANT ALL has no annotation — should fire SC-SEC02.
    // The second is preceded by an ignore annotation — should be suppressed.
    let sql = concat!(
        "GRANT ALL ON users TO bob;\n",
        "\n",
        "-- scythe-audit: ignore[SC-SEC02] reason=\"vetted\"\n",
        "GRANT ALL ON other TO alice;\n",
    );

    let findings = run_rules("smoke.sql", sql, &dialect, &catalog, &registry);

    let sec02: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-SEC02")
        .collect();
    assert_eq!(
        sec02.len(),
        1,
        "expected exactly 1 SC-SEC02 finding; got {}: {:#?}",
        sec02.len(),
        sec02
    );
}

// ---------------------------------------------------------------------------
// Test 2: user rule fires for custom function
// ---------------------------------------------------------------------------

#[test]
fn user_rule_fires_for_custom_function() {
    let dialect = SqlDialect::PostgreSQL;
    let catalog = Catalog::from_ddl_with_dialect(&[], &dialect).expect("empty catalog");

    let mut matcher_args = toml::Table::new();
    matcher_args.insert(
        "functions".to_string(),
        toml::Value::Array(vec![toml::Value::String("debug_print".to_string())]),
    );
    let spec = RuleSpec {
        id: "USER-001".to_string(),
        name: "no-debug-print".to_string(),
        category: RuleCategory::Security,
        severity: Severity::Error,
        dialects: vec![],
        cwe: vec![],
        description: "debug_print should not reach production".to_string(),
        message: "call to {func}".to_string(),
        matcher: "function_name_in_set".to_string(),
        matcher_args,
    };

    let mut registry = default_registry();
    let matcher_registry = MatcherRegistry::canonical();
    register_user_rules(
        &mut registry,
        &matcher_registry,
        &[(spec, "test".to_string())],
    )
    .expect("register_user_rules should succeed");

    let sql = "SELECT debug_print('hello');";
    let findings = run_rules("q.sql", sql, &dialect, &catalog, &registry);

    let user_findings: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "USER-001")
        .collect();
    assert!(
        !user_findings.is_empty(),
        "expected USER-001 to fire on debug_print call; findings: {:#?}",
        findings
    );
}

// ---------------------------------------------------------------------------
// Test 3: canonical ID collision returns AuditConfigError::InvalidRule
// ---------------------------------------------------------------------------

#[test]
fn user_rule_with_canonical_id_returns_error() {
    let mut registry = default_registry();
    let matcher_registry = MatcherRegistry::canonical();

    let mut matcher_args = toml::Table::new();
    matcher_args.insert(
        "functions".to_string(),
        toml::Value::Array(vec![toml::Value::String("bad_fn".to_string())]),
    );
    let spec = RuleSpec {
        id: "SC-SEC01".to_string(), // canonical — must be rejected
        name: "collision-test".to_string(),
        category: RuleCategory::Security,
        severity: Severity::Error,
        dialects: vec![],
        cwe: vec![],
        description: "test".to_string(),
        message: "msg".to_string(),
        matcher: "function_name_in_set".to_string(),
        matcher_args,
    };

    let source = "my_rules.toml".to_string();
    let result = register_user_rules(&mut registry, &matcher_registry, &[(spec, source)]);

    match result {
        Err(AuditConfigError::InvalidRule { path, rule_id, .. }) => {
            assert_eq!(rule_id, "SC-SEC01", "error should name the offending rule");
            assert!(
                path.contains("my_rules.toml"),
                "error should include the source path, got: {path}"
            );
        }
        other => panic!("expected InvalidRule error, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Test 4: missing USER- prefix returns error
// ---------------------------------------------------------------------------

#[test]
fn user_rule_without_user_prefix_returns_error() {
    let mut registry = default_registry();
    let matcher_registry = MatcherRegistry::canonical();

    let mut matcher_args = toml::Table::new();
    matcher_args.insert(
        "functions".to_string(),
        toml::Value::Array(vec![toml::Value::String("bad_fn".to_string())]),
    );
    let spec = RuleSpec {
        id: "NOPRE-001".to_string(), // no USER- prefix
        name: "prefix-test".to_string(),
        category: RuleCategory::Security,
        severity: Severity::Error,
        dialects: vec![],
        cwe: vec![],
        description: "test".to_string(),
        message: "msg".to_string(),
        matcher: "function_name_in_set".to_string(),
        matcher_args,
    };

    let result = register_user_rules(
        &mut registry,
        &matcher_registry,
        &[(spec, "rules.toml".to_string())],
    );

    assert!(
        result.is_err(),
        "missing USER- prefix must produce an error"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("USER-") || err_msg.contains("NOPRE-001"),
        "error message should mention the prefix requirement or the bad id: {err_msg}"
    );
}

// ---------------------------------------------------------------------------
// Test 5: a single inline suppression covering multiple rule IDs silences all
// of them on the next statement.
// ---------------------------------------------------------------------------

#[test]
fn multi_rule_suppression_silences_every_listed_id() {
    let dialect = SqlDialect::PostgreSQL;
    let catalog = Catalog::from_ddl_with_dialect(&[], &dialect).expect("empty catalog");
    let registry = default_registry();

    // Both GRANT ALL (SC-SEC02) and GRANT TO PUBLIC (SC-SEC03) would fire on
    // this statement. The annotation lists both — both must be silenced.
    let sql = concat!(
        "-- scythe-audit: ignore[SC-SEC02,SC-SEC03]\n",
        "GRANT ALL ON users TO PUBLIC;\n",
    );

    let findings = run_rules("multi.sql", sql, &dialect, &catalog, &registry);

    let sec02_03: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-SEC02" || f.rule_id == "SC-SEC03")
        .collect();
    assert!(
        sec02_03.is_empty(),
        "expected both SC-SEC02 and SC-SEC03 silenced; got {:#?}",
        sec02_03
    );
}

// ---------------------------------------------------------------------------
// Test 6: load_rules_from_file reads a separate TOML file referenced by the
// `extra_rules` mechanism; loaded specs can be registered as USER- rules.
// ---------------------------------------------------------------------------

#[test]
fn extra_rules_file_loads_and_registers() {
    let tmp = tempfile::TempDir::new().unwrap();
    let extra_path = tmp.path().join("extra.toml");
    let mut f = std::fs::File::create(&extra_path).unwrap();
    writeln!(
        f,
        r#"schema_version = 1

[[rule]]
id = "USER-EXTRA-001"
name = "no-extra-fn"
category = "security"
severity = "error"
description = "extra_rules test"
message = "call to {{func}}"
matcher = "function_name_in_set"

[rule.matcher_args]
functions = ["extra_fn"]
"#
    )
    .unwrap();
    drop(f);

    let specs = load_rules_from_file(&extra_path).expect("load_rules_from_file");
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].id, "USER-EXTRA-001");

    let mut registry = default_registry();
    let matcher_registry = MatcherRegistry::canonical();
    register_user_rules(
        &mut registry,
        &matcher_registry,
        &[(
            specs.into_iter().next().unwrap(),
            extra_path.display().to_string(),
        )],
    )
    .expect("register_user_rules");

    let dialect = SqlDialect::PostgreSQL;
    let catalog = Catalog::from_ddl_with_dialect(&[], &dialect).expect("empty catalog");
    let findings = run_rules("q.sql", "SELECT extra_fn();", &dialect, &catalog, &registry);
    assert!(
        findings.iter().any(|f| f.rule_id == "USER-EXTRA-001"),
        "expected USER-EXTRA-001 to fire; got: {:#?}",
        findings
    );
}

// ---------------------------------------------------------------------------
// Test 7: malformed TOML rule file produces a clear error containing the
// offending path.
// ---------------------------------------------------------------------------

#[test]
fn malformed_extra_rules_file_returns_error_with_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let bad_path = tmp.path().join("bad.toml");
    std::fs::write(&bad_path, "schema_version = \"not a number\"\n[[rule\n").unwrap();

    let err = load_rules_from_file(&bad_path).expect_err("malformed TOML must error");
    let msg = err.to_string();
    assert!(
        msg.contains("bad.toml") || msg.contains(bad_path.to_str().unwrap()),
        "error message should reference the offending path; got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Test 8: a Postgres-only rule (SC-SEC10) does not fire when the dialect is
// SQLite.
// ---------------------------------------------------------------------------

#[test]
fn pg_only_rule_skipped_on_non_pg_dialect() {
    let catalog_pg =
        Catalog::from_ddl_with_dialect(&[], &SqlDialect::PostgreSQL).expect("empty catalog");

    // CREATE FUNCTION … SECURITY DEFINER trips SC-SEC10 on Postgres.
    let sql = "CREATE FUNCTION elevate() RETURNS void LANGUAGE plpgsql SECURITY DEFINER AS $$ BEGIN END $$;";

    let registry = default_registry();
    let pg_findings = run_rules(
        "pg.sql",
        sql,
        &SqlDialect::PostgreSQL,
        &catalog_pg,
        &registry,
    );
    assert!(
        pg_findings.iter().any(|f| f.rule_id == "SC-SEC10"),
        "expected SC-SEC10 on Postgres; got: {:#?}",
        pg_findings
    );

    // The same CREATE FUNCTION won't even parse under SQLite, so we use a
    // statement that *would* fire SC-SEC11 (SET ROLE) on Postgres but is
    // skipped under SQLite because the rule declares dialects = ["postgres"].
    let set_role_sql = "SET ROLE admin;";
    let pg_set = run_rules(
        "pg2.sql",
        set_role_sql,
        &SqlDialect::PostgreSQL,
        &catalog_pg,
        &registry,
    );
    assert!(
        pg_set.iter().any(|f| f.rule_id == "SC-SEC11"),
        "expected SC-SEC11 on Postgres for SET ROLE; got: {:#?}",
        pg_set
    );

    let sqlite_set = run_rules(
        "sqlite.sql",
        set_role_sql,
        &SqlDialect::SQLite,
        &catalog_pg,
        &registry,
    );
    assert!(
        !sqlite_set.iter().any(|f| f.rule_id == "SC-SEC11"),
        "SC-SEC11 must not fire on SQLite (PG-only); got: {:#?}",
        sqlite_set
    );
}
