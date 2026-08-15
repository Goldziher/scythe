//! `.github/workflows/publish.yaml` pushes the workspace's publishable crates
//! to crates.io one at a time, and `cargo publish` resolves a path
//! dependency's `version` requirement against the registry rather than against
//! the workspace. So a crate must be published only after every workspace
//! crate it depends on, or it fails with "failed to select a version for the
//! requirement `<dep> = "^x.y.z"`".
//!
//! Nothing checked that. The 0.15.0 tag push published `scythe-backend` and
//! `scythe-core`, then died on `scythe-codegen`, because the hand-maintained
//! order in the workflow still listed `scythe-codegen` before `scythe-lint`
//! while `scythe-codegen/Cargo.toml` had gained a dependency on it. The failure
//! mode is the worst kind: it cannot be discovered before a tag exists, and by
//! the time it fires some crates are already published and immutable.
//!
//! This test derives the constraint from the same two sources the release
//! actually reads -- the `publish_crate <name>` lines in the workflow, and every
//! dependency table in each crate's manifest -- so a new inter-crate edge that
//! invalidates the order fails `cargo test` instead of a release.
//!
//! `[dev-dependencies]` are included deliberately; see
//! `workspace_dependencies_of`. A first draft of this test excluded them, on the
//! reasoning that a test-only edge cannot constrain publishing. That draft
//! passed on the exact manifest that had just broken the release -- the
//! `scythe-codegen` -> `scythe-lint` edge is a dev-dependency.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolving the repo root from CARGO_MANIFEST_DIR")
}

/// The crate names in `publish_crate <name>` order, read out of the publish
/// workflow. Deliberately parsed from the shell body rather than duplicated
/// here: a list this test keeps its own copy of would drift from the one the
/// release runs, which is the whole defect being guarded against.
fn publish_order(workflow_source: &str) -> Vec<String> {
    workflow_source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("publish_crate ").map(str::trim))
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// The workspace crates each crate under `crates/<name>` depends on in a way
/// that constrains publish order.
///
/// `[dev-dependencies]` count, and getting that wrong is what broke the 0.15.0
/// release: `scythe-codegen` names `scythe-lint` only under `[dev-dependencies]`
/// (`crates/scythe-codegen/Cargo.toml`), and `cargo publish` still failed with
/// "failed to select a version for the requirement `scythe-lint = "^0.15.0"`".
/// `cargo publish` strips the `path` from every dependency table and keeps the
/// `version`, then resolves the packaged manifest against the registry -- so a
/// dev-dependency carrying a `version` has to exist on crates.io just like a
/// normal one.
///
/// The distinction that does matter is the `version` key, not the table: a
/// path-only workspace dev-dependency is dropped entirely by `cargo publish`
/// and constrains nothing.
fn workspace_dependencies_of(root: &Path, crate_name: &str) -> BTreeSet<String> {
    const CONSTRAINING_TABLES: &[&str] = &["[dependencies]", "[dev-dependencies]", "[build-dependencies]"];

    let manifest_path = root.join("crates").join(crate_name).join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", manifest_path.display()));

    let mut dependencies = BTreeSet::new();
    let mut in_constraining_table = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_constraining_table = CONSTRAINING_TABLES.contains(&trimmed);
            continue;
        }
        if !in_constraining_table {
            continue;
        }
        let Some((name, spec)) = trimmed.split_once(" = ") else {
            continue;
        };
        let name = name.trim();
        if name.starts_with("scythe-") && spec.contains("version") {
            dependencies.insert(name.to_string());
        }
    }
    dependencies
}

#[test]
fn publish_order_is_a_valid_topological_sort() {
    let root = repo_root();
    let workflow_path = root.join(".github/workflows/publish.yaml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", workflow_path.display()));

    let order = publish_order(&workflow);
    assert!(
        order.len() >= 2,
        "found {} `publish_crate` line(s) in {} -- the parser is not matching the workflow's \
         actual shape, so this test would pass vacuously",
        order.len(),
        workflow_path.display()
    );

    let listed: BTreeSet<&str> = order.iter().map(String::as_str).collect();
    assert_eq!(
        listed.len(),
        order.len(),
        "a crate is listed twice in the publish order: {order:?}"
    );

    let mut position = BTreeMap::new();
    for (index, name) in order.iter().enumerate() {
        position.insert(name.as_str(), index);
    }

    let mut violations = Vec::new();
    for (index, name) in order.iter().enumerate() {
        for dependency in workspace_dependencies_of(&root, name) {
            let Some(&dependency_index) = position.get(dependency.as_str()) else {
                // ~keep A workspace crate that is never published (scythe-conformance, the
                // tools/) cannot constrain the order, but a *published* crate depending on
                // an unpublished one would break the release, so say so rather than skip.
                violations.push(format!(
                    "{name} (published) depends on {dependency}, which no `publish_crate` line \
                     publishes -- the release would resolve it against crates.io and fail"
                ));
                continue;
            };
            if dependency_index > index {
                violations.push(format!(
                    "{name} is published at position {index} but depends on {dependency} at \
                     position {dependency_index} -- cargo publish resolves that dependency's \
                     version against crates.io, where it does not exist yet"
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "the `publish_crate` order in {} is not a valid topological sort of the workspace \
         dependency graph. Reorder the calls; every crate must follow the crates it depends \
         on:\n{}",
        workflow_path.display(),
        violations.join("\n")
    );
}
