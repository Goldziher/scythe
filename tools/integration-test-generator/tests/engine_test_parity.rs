//! Parity gate for GH #196/#195: `templates/java.java.jinja` and
//! `templates/kotlin.kt.jinja` each carry one whole duplicated test program per top-level
//! `{% if %}` / `{% elif %}` branch -- one per SQL engine (`engine == "mariadb"`), plus a few
//! conditioned on driver or backend instead (`driver == "r2dbc" and engine == "postgresql"`,
//! `backend == "kotlin-exposed"`). Nothing ever compared what one branch tests against what
//! another does, so they drifted -- the redshift branches ended up with fewer than half the test
//! functions the postgresql branch has, in both templates.
//!
//! This test measures every top-level branch's test-function count against the `postgresql`
//! (SQL-engine) branch of the same template and fails if a branch is missing a test the
//! postgresql branch has, unless `test-parity-exemptions.txt` names that exact
//! `(template, branch, test)` triple with a reason. The allowlist ratchets in both directions
//! (see that file's header): a gap with no exemption fails as a regression, and a stale exemption
//! whose gap has been closed fails too, so the list can only shrink and can never be papered over
//! with a percentage.
//!
//! Branch discovery recognises *every* top-level `if`/`elif`, not just ones
//! literally spelled `engine == "..."`: `derive_branch_key` extracts every double-quoted string
//! literal a condition compares against, in source order, and joins them with `-`. That keys
//! `engine == "mariadb"` as `mariadb` (unchanged), `driver == "r2dbc" and engine == "postgresql"`
//! as `r2dbc-postgresql`, `driver == "r2dbc" and (engine == "mysql" or engine == "mariadb")` as
//! `r2dbc-mysql-mariadb`, and `backend == "kotlin-exposed"` as `kotlin-exposed`, without
//! hardcoding any of those five conditions -- a future top-level branch is picked up as long as
//! its condition contains a quoted literal to key on.
//!
//! Two checks guarantee the gate actually measures what it claims to:
//!
//!   * **no silent branch collision.** If two top-level branch starts derive the same key,
//!     `branch_test_names` panics naming both line numbers, instead of the previous behaviour of
//!     quietly overwriting one branch's measured test set with the other's via `BTreeMap::insert`.
//!     This is what catches a nested column-0 `{% if engine == "..." %}` that looks like a
//!     top-level branch to a line-based splitter but is really shadowing a real one.
//!   * **no orphaned test function.** `assert_no_test_falls_outside_every_branch` finds every
//!     test-function definition in the whole file and fails, naming the file:line and function
//!     name of each one, if any of them falls outside every window `branch_test_names` produces --
//!     i.e. its enclosing top-level condition was never recognised as a branch start at all. This
//!     is what would have caught GH #195's defect 1: 21 java test functions and 35 kotlin ones
//!     sitting in `driver == "r2dbc"`/`backend == "kotlin-exposed"` branches the old splitter never
//!     saw, so they were excluded from every parity comparison without a trace.
//!
//! What counts as a "test function": a Java `private static void test...(` method or a Kotlin
//! top-level `fun test...(` function inside a branch's line range. Both templates hand-roll their
//! own pass/fail harness rather than using JUnit, so there is no `@Test` annotation to grep for.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// One exemption: a specific top-level branch is missing a specific postgresql-branch test
/// function, with a reason it is not (yet, or ever) expected to have it. `engine` holds the
/// branch's derived key (see `derive_branch_key`) -- for most branches that is a literal SQL
/// engine name (`mariadb`), but for driver/backend-conditioned branches it is the joined key
/// (`r2dbc-postgresql`, `kotlin-exposed`).
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
                "{}:{}: expected `<template>|<branch>|<test> : <reason>`, got: {line}",
                path.display(),
                line_number + 1
            )
        });
        let mut fields = key_part.trim().split('|');
        let (Some(template), Some(engine), Some(test_name), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            panic!(
                "{}:{}: expected exactly `<template>|<branch>|<test>` before ':', got: {key_part}",
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

/// Derives a stable branch key from a `{% if ... %}` / `{% elif ... %}` condition (with the
/// leading `if `/`elif ` already stripped) by extracting every double-quoted string literal it
/// compares against, in source order, and joining them with `-`. Returns `None` for a condition
/// with no quoted literal at all -- not a branch this gate tracks.
///
/// `engine == "mariadb"` keys as `mariadb`; `driver == "r2dbc" and engine == "postgresql"` keys
/// as `r2dbc-postgresql`; `driver == "r2dbc" and (engine == "mysql" or engine == "mariadb")` keys
/// as `r2dbc-mysql-mariadb`; `backend == "kotlin-exposed"` keys as `kotlin-exposed`. This is
/// derived from the condition text rather than hardcoded to today's branches, so a new
/// driver/backend-conditioned branch is picked up automatically as long as its condition contains
/// a quoted literal.
fn derive_branch_key(condition: &str) -> Option<String> {
    let mut literals = Vec::new();
    let mut rest = condition;
    while let Some(start) = rest.find('"') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('"') else { break };
        literals.push(rest[..end].to_string());
        rest = &rest[end + 1..];
    }
    (!literals.is_empty()).then(|| literals.join("-"))
}

/// Finds every top-level (column-0) `{% if ... %}` / `{% elif ... %}` branch start in `lines`, in
/// file order, as `(zero_based_line_index, derived_key)`. A line only counts if it begins with a
/// literal `{%-`/`{%` at column 0 -- a nested block written with any leading whitespace before the
/// tag is correctly excluded, since `strip_prefix` only matches at the very start of the line --
/// and its condition yields a key from `derive_branch_key`.
fn top_level_branch_starts(lines: &[&str]) -> Vec<(usize, String)> {
    let mut branch_starts = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(stripped) = line.strip_prefix("{%-").or_else(|| line.strip_prefix("{%")) else {
            continue;
        };
        let trimmed = stripped.trim_start();
        let Some(condition) = trimmed.strip_prefix("if ").or_else(|| trimmed.strip_prefix("elif ")) else {
            continue;
        };
        let Some(key) = derive_branch_key(condition) else {
            continue;
        };
        branch_starts.push((index, key));
    }
    branch_starts
}

/// Panics if two top-level branch starts derive the same key. This is the load-bearing check that
/// catches a nested column-0 `{% if ... %}`/`{% elif ... %}` silently shadowing a real top-level
/// branch, which previously caused `BTreeMap::insert` to overwrite the real branch's measured test
/// coverage with the nested block's, without a trace.
fn panic_on_duplicate_branch_keys(template_filename: &str, branch_starts: &[(usize, String)]) {
    let mut first_seen: BTreeMap<&str, usize> = BTreeMap::new();
    for (index, key) in branch_starts {
        if let Some(&first_index) = first_seen.get(key.as_str()) {
            panic!(
                "{template_filename}:{}: top-level branch `{key}` collides with the branch already \
                 started at {template_filename}:{} -- a nested column-0 `{{% if ... %}}` (or `{{% \
                 elif ... %}}`) is shadowing a real top-level branch and would silently overwrite \
                 its measured test coverage. Indent the nested block so it no longer starts at \
                 column 0, or give it a condition that derives a distinct key.",
                index + 1,
                first_index + 1,
            );
        }
        first_seen.insert(key.as_str(), *index);
    }
}

/// The test-function names found per top-level branch key, paired with the branch-start list they
/// were measured against (`(zero_based_line_index, derived_key)`, `__end__` sentinel included).
type BranchTestNames = (BTreeMap<String, BTreeSet<String>>, Vec<(usize, String)>);

/// Splits a template's source into `(branch_key, test_function_names)` per top-level branch (see
/// `top_level_branch_starts` and `derive_branch_key`), using `test_pattern` to find test-function
/// definitions within each branch's line range. Returns the map alongside the branch-start list
/// (with an `__end__` sentinel appended at `lines.len()`) so callers can also run
/// `assert_no_test_falls_outside_every_branch`. Panics via `panic_on_duplicate_branch_keys` if two
/// branch starts derive the same key.
fn branch_test_names(template_filename: &str, lines: &[&str], test_pattern: &str) -> BranchTestNames {
    let mut branch_starts = top_level_branch_starts(lines);
    panic_on_duplicate_branch_keys(template_filename, &branch_starts);
    branch_starts.push((lines.len(), "__end__".to_string()));

    let mut result = BTreeMap::new();
    for window in branch_starts.windows(2) {
        let (start, key) = &window[0];
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
        result.insert(key.clone(), names);
    }
    (result, branch_starts)
}

/// Every test-function definition in the whole file, as `(zero_based_line_index, test_name)`,
/// independent of branch.
fn all_test_occurrences(lines: &[&str], test_pattern: &str) -> Vec<(usize, String)> {
    let mut occurrences = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(pattern_index) = line.find(test_pattern) else {
            continue;
        };
        let after = &line[pattern_index + test_pattern.len()..];
        let Some(name_end) = after.find('(') else {
            continue;
        };
        if name_end == 0 {
            continue;
        }
        occurrences.push((index, format!("test{}", &after[..name_end])));
    }
    occurrences
}

/// Fails, naming each offender's file:line and function name, if any test function `occurrences`
/// found falls outside every window `branch_starts` (with its `__end__` sentinel already
/// appended) produces -- i.e. its enclosing top-level branch condition was never recognised as a
/// branch start at all, so `branch_test_names` silently excluded it from every comparison.
fn assert_no_test_falls_outside_every_branch(
    template_filename: &str,
    occurrences: &[(usize, String)],
    branch_starts: &[(usize, String)],
) {
    let orphans: Vec<String> = occurrences
        .iter()
        .filter(|(line_index, _)| {
            !branch_starts
                .windows(2)
                .any(|window| *line_index >= window[0].0 && *line_index < window[1].0)
        })
        .map(|(line_index, name)| format!("{template_filename}:{}: {name}", line_index + 1))
        .collect();

    assert!(
        orphans.is_empty(),
        "test function(s) fall outside every top-level branch range in {template_filename} -- the \
         branch-splitting logic in engine_test_parity.rs does not recognise the condition that \
         starts their enclosing block (or there is no enclosing branch at all), so they are \
         silently excluded from every parity comparison:\n{}",
        orphans.join("\n")
    );
}

fn check_template_parity(template_filename: &str, template_key: &str, test_pattern: &str) {
    let path = templates_dir().join(template_filename);
    let source = fs::read_to_string(&path).unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
    let lines: Vec<&str> = source.lines().collect();

    let (branches, branch_starts) = branch_test_names(template_filename, &lines, test_pattern);
    let occurrences = all_test_occurrences(&lines, test_pattern);
    assert_no_test_falls_outside_every_branch(template_filename, &occurrences, &branch_starts);

    let postgresql_tests = branches
        .get("postgresql")
        .unwrap_or_else(|| panic!("{template_filename} has no postgresql branch to compare against"));

    let exemptions = load_exemptions();
    let mut used_exemptions = BTreeSet::new();
    let mut regressions = Vec::new();

    for (branch_key, tests) in &branches {
        if branch_key == "postgresql" || branch_key == "__end__" {
            continue;
        }
        for test_name in postgresql_tests {
            if tests.contains(test_name) {
                continue;
            }
            let key = ExemptionKey {
                template: template_key.to_string(),
                engine: branch_key.clone(),
                test_name: test_name.clone(),
            };
            if exemptions.contains(&key) {
                used_exemptions.insert(key);
            } else {
                regressions.push(format!(
                    "{template_key}|{branch_key}|{test_name}: present in the postgresql branch, \
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

#[cfg(test)]
mod branch_splitter_unit_tests {
    use super::*;

    #[test]
    fn derive_branch_key_returns_none_when_condition_has_no_quoted_literal() {
        assert_eq!(derive_branch_key("some_flag -%}"), None);
    }

    #[test]
    fn derive_branch_key_keeps_a_single_literal_as_is() {
        assert_eq!(
            derive_branch_key("engine == \"postgresql\" -%}"),
            Some("postgresql".to_string())
        );
    }

    #[test]
    fn derive_branch_key_joins_a_driver_and_engine_literal() {
        assert_eq!(
            derive_branch_key("driver == \"r2dbc\" and engine == \"postgresql\" -%}"),
            Some("r2dbc-postgresql".to_string())
        );
    }

    #[test]
    fn derive_branch_key_joins_multiple_literals_in_source_order() {
        assert_eq!(
            derive_branch_key("driver == \"r2dbc\" and (engine == \"mysql\" or engine == \"mariadb\") -%}"),
            Some("r2dbc-mysql-mariadb".to_string())
        );
    }

    #[test]
    fn top_level_branch_starts_ignores_a_nested_block_indented_off_column_zero() {
        let source = "{%- if engine == \"postgresql\" -%}\n    {%- if engine == \"mariadb\" %}\nfun testX() {}\n\
                       \x20   {%- endif %}\n{%- endif %}";
        let lines: Vec<&str> = source.lines().collect();
        assert_eq!(top_level_branch_starts(&lines), vec![(0, "postgresql".to_string())]);
    }

    #[test]
    #[should_panic(
        expected = "synthetic.jinja:3: top-level branch `mariadb` collides with the branch already \
                                started at synthetic.jinja:1"
    )]
    fn branch_test_names_panics_on_duplicate_derived_key() {
        let source = "{%- if engine == \"mariadb\" %}\nfun testOne() {}\n{%- elif engine == \"mariadb\" -%}\n\
                       fun testTwo() {}\n{%- endif %}";
        let lines: Vec<&str> = source.lines().collect();
        branch_test_names("synthetic.jinja", &lines, "fun test");
    }

    #[test]
    #[should_panic(expected = "synthetic.jinja:1: testOrphan")]
    fn assert_no_test_falls_outside_every_branch_panics_when_a_test_precedes_every_branch_start() {
        let source = "fun testOrphan() {}\n{%- if engine == \"postgresql\" -%}\nfun testInBranch() {}\n\
                       {%- endif %}";
        let lines: Vec<&str> = source.lines().collect();
        let (_, branch_starts) = branch_test_names("synthetic.jinja", &lines, "fun test");
        let occurrences = all_test_occurrences(&lines, "fun test");
        assert_no_test_falls_outside_every_branch("synthetic.jinja", &occurrences, &branch_starts);
    }
}
