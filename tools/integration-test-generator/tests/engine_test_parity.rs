//! Parity gate for GH #196/#195: `templates/java.java.jinja` and
//! `templates/kotlin.kt.jinja` each carry one whole duplicated test program
//! per SQL engine (`{% if engine == "..." %}` / `{% elif engine == "..." %}`
//! branch). Nothing ever compared what one branch tests against what another
//! does, so they drifted -- the redshift branches ended up with fewer than
//! half the test functions the postgresql branch has, in both templates.
//!
//! This test measures every branch's test-function count against the
//! postgresql branch of the same template and fails if a branch is missing a
//! test the postgresql branch has, unless `test-parity-exemptions.txt` names
//! that exact `(template, engine, test)` triple with a reason. The allowlist
//! ratchets in both directions (see that file's header): a gap with no
//! exemption fails as a regression, and a stale exemption whose gap has been
//! closed fails too, so the list can only shrink and can never be papered
//! over with a percentage.
//!
//! What counts as a "test function": a Java `private static void test...(`
//! method or a Kotlin top-level `fun test...(` function inside the branch's
//! line range. Both templates hand-roll their own pass/fail harness rather
//! than using JUnit, so there is no `@Test` annotation to grep for.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// One exemption: a specific branch is missing a specific postgresql-branch
/// test function, with a reason it is not (yet, or ever) expected to have it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ExemptionKey {
    template: String,
    engine: String,
    test_name: String,
}

fn templates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("templates")
}

fn load_exemptions() -> BTreeSet<ExemptionKey> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-parity-exemptions.txt");
    let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));

    let mut exemptions = BTreeSet::new();
    for (line_number, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key_part, _reason) = line.split_once(':').unwrap_or_else(|| {
            panic!(
                "{}:{}: expected `<template>|<engine>|<test> : <reason>`, got: {line}",
                path.display(),
                line_number + 1
            )
        });
        let mut fields = key_part.trim().split('|');
        let (Some(template), Some(engine), Some(test_name), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            panic!(
                "{}:{}: expected exactly `<template>|<engine>|<test>` before ':', got: {key_part}",
                path.display(),
                line_number + 1
            );
        };
        let inserted = exemptions.insert(ExemptionKey {
            template: template.to_string(),
            engine: engine.to_string(),
            test_name: test_name.trim().to_string(),
        });
        assert!(
            inserted,
            "{}:{}: duplicate exemption entry for {key_part}",
            path.display(),
            line_number + 1
        );
    }
    exemptions
}

/// Splits a template's source into `(engine, test_function_names)` per
/// top-level `{% if engine == "X" %}` / `{% elif engine == "Y" %}` branch,
/// using `test_pattern` to find test-function definitions within each
/// branch's line range.
fn branch_test_names(template_source: &str, test_pattern: &str) -> BTreeMap<String, BTreeSet<String>> {
    let lines: Vec<&str> = template_source.lines().collect();

    let mut branch_starts: Vec<(usize, String)> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start_matches("{%-").trim_start_matches("{%").trim();
        let Some(rest) = trimmed
            .strip_prefix("if engine == \"")
            .or_else(|| trimmed.strip_prefix("elif engine == \""))
        else {
            continue;
        };
        let Some(end) = rest.find('"') else { continue };
        branch_starts.push((index, rest[..end].to_string()));
    }
    branch_starts.push((lines.len(), "__end__".to_string()));

    let mut result = BTreeMap::new();
    for window in branch_starts.windows(2) {
        let (start, engine) = &window[0];
        let (end, _) = &window[1];
        let mut names = BTreeSet::new();
        for line in &lines[*start..*end] {
            if let Some(index) = line.find(test_pattern) {
                let after = &line[index + test_pattern.len()..];
                let name_end = after.find('(').unwrap_or(0);
                if name_end > 0 {
                    names.insert(format!("test{}", &after[..name_end]));
                }
            }
        }
        result.insert(engine.clone(), names);
    }
    result
}

fn check_template_parity(template_filename: &str, template_key: &str, test_pattern: &str) {
    let path = templates_dir().join(template_filename);
    let source = fs::read_to_string(&path).unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));

    let branches = branch_test_names(&source, test_pattern);
    let postgresql_tests = branches
        .get("postgresql")
        .unwrap_or_else(|| panic!("{template_filename} has no postgresql branch to compare against"));

    let exemptions = load_exemptions();
    let mut used_exemptions = BTreeSet::new();
    let mut regressions = Vec::new();

    for (engine, tests) in &branches {
        if engine == "postgresql" || engine == "__end__" {
            continue;
        }
        for test_name in postgresql_tests {
            if tests.contains(test_name) {
                continue;
            }
            let key = ExemptionKey {
                template: template_key.to_string(),
                engine: engine.clone(),
                test_name: test_name.clone(),
            };
            if exemptions.contains(&key) {
                used_exemptions.insert(key);
            } else {
                regressions.push(format!(
                    "{template_key}|{engine}|{test_name}: present in the postgresql branch, \
                     missing here, and not in test-parity-exemptions.txt"
                ));
            }
        }
    }

    assert!(
        regressions.is_empty(),
        "engine test parity regression(s) in {template_filename} -- add an exemption with a \
         reason to test-parity-exemptions.txt, or port the test:\n{}",
        regressions.join("\n")
    );

    let stale: Vec<&ExemptionKey> = exemptions
        .iter()
        .filter(|key| key.template == template_key && !used_exemptions.contains(*key))
        .collect();
    assert!(
        stale.is_empty(),
        "stale exemption(s) in test-parity-exemptions.txt for {template_filename} -- the branch \
         now has this test, delete the line:\n{}",
        stale
            .iter()
            .map(|key| format!("{}|{}|{}", key.template, key.engine, key.test_name))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn java_engine_branches_match_postgresql_test_coverage() {
    check_template_parity("java.java.jinja", "java", "private static void test");
}

#[test]
fn kotlin_engine_branches_match_postgresql_test_coverage() {
    check_template_parity("kotlin.kt.jinja", "kotlin", "fun test");
}
