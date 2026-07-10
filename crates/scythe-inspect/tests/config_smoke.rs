//! Config smoke tests — verify that `parse_inspect_section` + `CheckRegistry`
//! correctly apply `[inspect]` config knobs (severity overrides, user checks,
//! extra_rules) without a database connection.
//!
//! Tests use `tempfile::TempDir` for isolation; each temp dir is dropped at the
//! end of the test, so cleanup is automatic.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;

use scythe_inspect::{CheckRegistry, parse_inspect_section};
use scythe_lint::Severity;

/// Write `content` to a file named `scythe.toml` inside a freshly created
/// `TempDir`.  Returns the guard (keep alive for the test) and the path to
/// the config file.
fn write_scythe_toml(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let config_path = dir.path().join("scythe.toml");
    let mut f = std::fs::File::create(&config_path).expect("create scythe.toml");
    f.write_all(content.as_bytes()).expect("write");
    (dir, config_path)
}

/// `[inspect.severity_overrides] "SC-INS04" = "off"` — the registry built
/// from that config must NOT include SC-INS04.
#[test]
fn severity_override_off_silences_check() {
    let toml = r#"
[inspect.severity_overrides]
"SC-INS04" = "off"
"#;
    let (_dir, config_path) = write_scythe_toml(toml);

    let cfg = parse_inspect_section(&config_path)
        .expect("parse must succeed")
        .expect("must have [inspect] block");

    let mut registry = CheckRegistry::canonical();
    registry.apply_severity_overrides(&cfg.severity_overrides);

    assert!(
        registry.get("SC-INS04").is_none(),
        "SC-INS04 must be removed by severity override 'off'"
    );
    assert!(
        registry.get("SC-INS01").is_some(),
        "SC-INS01 must still be in the registry"
    );
    let ids: Vec<_> = registry.for_engine("postgres").map(|s| s.id.as_str()).collect();
    assert!(
        !ids.contains(&"SC-INS04"),
        "SC-INS04 must not appear in for_engine('postgres'); ids: {ids:?}"
    );
}

/// `[inspect.severity_overrides] "SC-INS01" = "error"` — after the override
/// the SC-INS01 spec's severity must be `Error`.
#[test]
fn severity_override_warn_to_error_changes_severity() {
    let toml = r#"
[inspect.severity_overrides]
"SC-INS01" = "error"
"#;
    let (_dir, config_path) = write_scythe_toml(toml);

    let cfg = parse_inspect_section(&config_path)
        .expect("parse must succeed")
        .expect("must have [inspect] block");

    let mut registry = CheckRegistry::canonical();
    registry.apply_severity_overrides(&cfg.severity_overrides);

    let spec = registry
        .get("SC-INS01")
        .expect("SC-INS01 must still exist after severity override");

    assert_eq!(
        spec.severity,
        Severity::Error,
        "SC-INS01 must have severity Error after override; got: {:?}",
        spec.severity
    );
}

/// An inline `[[inspect.check]]` with a valid `USER-INS-` id must be loaded
/// and appear in `registry.for_engine("postgres")`.
#[test]
fn user_check_appears_in_list() {
    let toml = r#"
[[inspect.check]]
id          = "USER-INS-001"
name        = "no-comments-on-tables"
category    = "schema"
severity    = "warn"
engines     = ["postgres"]
description = "tables must have COMMENT ON TABLE"
message     = "table {schema_name}.{table_name} has no comment"
sql         = "SELECT n.nspname AS schema_name, c.relname AS table_name FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind = 'r'"
"#;
    let (_dir, config_path) = write_scythe_toml(toml);

    let cfg = parse_inspect_section(&config_path)
        .expect("parse must succeed")
        .expect("must have [inspect] block");

    assert_eq!(cfg.check.len(), 1, "one inline check expected");
    assert_eq!(cfg.check[0].id, "USER-INS-001");

    let registry = CheckRegistry::canonical().with_inline_checks(cfg.check);

    assert!(
        registry.get("USER-INS-001").is_some(),
        "USER-INS-001 must be in the registry after with_inline_checks"
    );

    let ids: Vec<_> = registry.for_engine("postgres").map(|s| s.id.as_str()).collect();
    assert!(
        ids.contains(&"USER-INS-001"),
        "USER-INS-001 must appear in for_engine('postgres'); ids: {ids:?}"
    );
    assert!(
        ids.contains(&"SC-INS01"),
        "canonical SC-INS01 must still appear; ids: {ids:?}"
    );
}

/// An inline `[[inspect.check]]` whose id does NOT start with `USER-INS-` must
/// cause `parse_inspect_section` to return a `ConfigError::InvalidCheck` that
/// mentions the required prefix.
#[test]
fn invalid_user_check_id_errors_clearly() {
    let toml = r#"
[[inspect.check]]
id          = "BAD-001"
name        = "bad-check"
category    = "schema"
severity    = "warn"
engines     = ["postgres"]
description = "test"
message     = "msg"
sql         = "SELECT 1 AS x"
"#;
    let (_dir, config_path) = write_scythe_toml(toml);

    let err = parse_inspect_section(&config_path).expect_err("must fail for invalid user check id");

    let msg = err.to_string();
    assert!(
        msg.contains("USER-INS-") || msg.contains("BAD-001"),
        "error must mention USER-INS- prefix or the offending id; got: {msg}"
    );
}

/// `extra_rules = ["./extra.toml"]` — the path must be resolved relative to
/// `scythe.toml`'s directory, NOT the current working directory.
///
/// This test proves it by placing `scythe.toml` and `extra.toml` in a
/// subdirectory (`cfg/`) under the temp root.  `parse_inspect_section` is
/// called with the absolute path to `cfg/scythe.toml`, which should succeed
/// because it resolves `./extra.toml` relative to `cfg/`.  If it wrongly
/// resolved relative to CWD the test would fail because no `extra.toml`
/// exists in the CWD.
#[test]
fn extra_rules_file_loaded_relative_to_config_dir() {
    let outer = tempfile::TempDir::new().expect("outer temp dir");
    let cfg_dir = outer.path().join("cfg");
    std::fs::create_dir(&cfg_dir).expect("create cfg subdir");

    let extra_toml = r#"schema_version = 1

[[check]]
id          = "USER-INS-002"
name        = "extra-user-check"
category    = "schema"
severity    = "warn"
engines     = ["postgres"]
description = "extra check loaded from extra.toml"
message     = "schema {schema_name} table {table_name}"
sql         = "SELECT n.nspname AS schema_name, c.relname AS table_name FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind = 'r'"
"#;
    std::fs::write(cfg_dir.join("extra.toml"), extra_toml).expect("write extra.toml");

    let scythe_toml = r#"
[inspect]
extra_rules = ["./extra.toml"]
"#;
    let config_path = cfg_dir.join("scythe.toml");
    std::fs::write(&config_path, scythe_toml).expect("write scythe.toml");

    let cfg = parse_inspect_section(&config_path)
        .expect("parse must succeed")
        .expect("must have [inspect] block");

    assert_eq!(cfg.extra_rules.len(), 1, "one extra_rules entry expected");
    assert!(
        cfg.extra_rules[0].contains("extra.toml"),
        "resolved path must mention extra.toml; got: {}",
        cfg.extra_rules[0]
    );

    let registry = CheckRegistry::canonical()
        .with_user_checks(Path::new(&cfg.extra_rules[0]))
        .expect("with_user_checks must succeed");

    assert!(
        registry.get("USER-INS-002").is_some(),
        "USER-INS-002 from extra.toml must be in the registry"
    );
    assert!(
        registry.get("SC-INS01").is_some(),
        "SC-INS01 must still be present alongside extra rules"
    );
}

/// Verify that multiple overrides can be applied in a single pass: one check
/// removed, one bumped to error.
#[test]
fn multiple_overrides_applied_together() {
    let toml = r#"
[inspect.severity_overrides]
"SC-INS01" = "error"
"SC-INS03" = "off"
"#;
    let (_dir, config_path) = write_scythe_toml(toml);

    let cfg = parse_inspect_section(&config_path)
        .expect("parse must succeed")
        .expect("must have [inspect] block");

    let mut overrides: HashMap<String, Severity> = HashMap::new();
    for (k, v) in &cfg.severity_overrides {
        overrides.insert(k.clone(), *v);
    }

    let mut registry = CheckRegistry::canonical();
    registry.apply_severity_overrides(&overrides);

    assert_eq!(
        registry.get("SC-INS01").map(|s| s.severity),
        Some(Severity::Error),
        "SC-INS01 must be Error"
    );
    assert!(registry.get("SC-INS03").is_none(), "SC-INS03 must be removed ('off')");
    assert!(
        registry.get("SC-INS02").is_some(),
        "SC-INS02 (untouched) must still be present"
    );
}
