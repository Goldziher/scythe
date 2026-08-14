//! Structural completeness checks for integration-test coverage (issues #126,
//! #134, #135).
//!
//! The defect these close is not "DuckDB has no project" — it is that the set
//! of shipped backends and the set of exercised backends were two independent
//! derivations of one fact that nothing ever compared. Seventeen manifests had
//! drifted out of coverage and only an audit noticed.
//!
//! So this file never hand-writes either list. `manifests/` is enumerated from
//! disk, the covered set is enumerated from the committed `scythe.toml` files,
//! and — critically — a project's `(backend, engine)` is mapped onto the
//! manifest it actually selects by calling the *real* resolver,
//! `scythe_codegen::backends::get_backend`. That matters because the mapping is
//! not the identity: `csharp-mysqlconnector` on engine `mariadb` resolves to
//! the `mysql` manifest, there being no mariadb manifest for it. Re-deriving
//! that fallback table here would reintroduce exactly the "two derivations,
//! never cross-checked" shape these checks exist to kill, so the resolver is
//! asked instead.
//!
//! Gaps that cannot be closed in a given pass go in
//! `coverage-exemptions.txt`, which ratchets in both directions: a manifest
//! with no project and no exemption fails, and an exemption whose manifest has
//! since gained a project also fails as a stale entry. There is no tolerance,
//! no percentage and no soft mode — every entry is one backend with one
//! written reason, and the list can only shrink.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use scythe_codegen::backends::get_backend;
use serde::Deserialize;

/// A manifest's identity: the `[backend] name` and `[backend] engine` it
/// declares. This is the unit of coverage — not the filename, because a
/// project reaches a manifest through the resolver, which keys on this pair.
type BackendPair = (String, String);

#[derive(Debug, Deserialize)]
struct ManifestFile {
    backend: ManifestBackend,
}

#[derive(Debug, Deserialize)]
struct ManifestBackend {
    name: String,
    engine: String,
}

#[derive(Debug, Deserialize)]
struct ScytheConfig {
    #[serde(default)]
    sql: Vec<SqlBlock>,
}

#[derive(Debug, Deserialize)]
struct SqlBlock {
    engine: String,
    /// `gen` is a reserved keyword in edition 2024, hence the rename.
    #[serde(default, rename = "gen")]
    generators: Vec<GenBlock>,
}

#[derive(Debug, Deserialize)]
struct GenBlock {
    backend: String,
}

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is tools/integration-test-generator.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate is two levels below the repo root")
        .to_path_buf()
}

fn read_to_string(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// Sorted list of paths matching `*.toml` directly inside `dir`.
fn toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    files.sort();
    files
}

/// Every backend manifest that ships, keyed by the pair the resolver uses.
/// The value is the filename, so failure messages can name the file to edit.
fn shipped_manifests() -> BTreeMap<BackendPair, String> {
    let dir = repo_root().join("crates/scythe-codegen/manifests");
    let mut manifests = BTreeMap::new();
    for path in toml_files(&dir) {
        let parsed: ManifestFile = toml::from_str(&read_to_string(&path))
            .unwrap_or_else(|e| panic!("parsing manifest {}: {e}", path.display()));
        let file = path
            .file_name()
            .expect("manifest path has a filename")
            .to_string_lossy()
            .into_owned();
        let previous = manifests.insert((parsed.backend.name, parsed.backend.engine), file.clone());
        assert!(
            previous.is_none(),
            "two manifests declare the same (name, engine); {file} collides with {}",
            previous.unwrap_or_default()
        );
    }
    assert!(!manifests.is_empty(), "no manifests found under {}", dir.display());
    manifests
}

/// Directory names of every committed integration project, i.e. every
/// directory under `integration_tests/` holding a `scythe.toml`.
fn integration_projects() -> Vec<(String, PathBuf)> {
    let dir = repo_root().join("integration_tests");
    let mut projects: Vec<(String, PathBuf)> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter_map(|path| {
            let config = path.join("scythe.toml");
            if !config.is_file() {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().into_owned();
            Some((name, config))
        })
        .collect();
    projects.sort();
    assert!(
        !projects.is_empty(),
        "no integration projects found under {}",
        dir.display()
    );
    projects
}

/// The manifests actually exercised by a committed project, resolved through
/// the same code path `scythe generate` takes.
fn covered_manifests() -> BTreeMap<BackendPair, BTreeSet<String>> {
    let mut covered: BTreeMap<BackendPair, BTreeSet<String>> = BTreeMap::new();
    for (project, config_path) in integration_projects() {
        let config: ScytheConfig = toml::from_str(&read_to_string(&config_path))
            .unwrap_or_else(|e| panic!("parsing {}: {e}", config_path.display()));
        for block in &config.sql {
            for generated in &block.generators {
                // A project naming a backend/engine combination the resolver
                // rejects is itself a defect, and a silent one today: it would
                // otherwise just fail to contribute coverage and read as an
                // uncovered manifest somewhere else.
                let backend = get_backend(&generated.backend, &block.engine).unwrap_or_else(|e| {
                    panic!(
                        "integration project '{project}' requests backend '{}' on engine '{}', \
                         which does not resolve to any manifest: {e}",
                        generated.backend, block.engine
                    )
                });
                let meta = &backend.manifest().backend;
                covered
                    .entry((meta.name.clone(), meta.engine.clone()))
                    .or_default()
                    .insert(project.clone());
            }
        }
    }
    covered
}

/// Parse a `<key> : <reason>` exemption file. Blank lines and `#` comments are
/// ignored. A reason is mandatory — an entry without one is rejected, because
/// an unexplained exemption is indistinguishable from an oversight.
fn load_exemptions(path: &Path, section: &str) -> BTreeMap<String, String> {
    let contents = read_to_string(path);
    let mut in_section = false;
    let mut entries = BTreeMap::new();
    for raw in contents.lines() {
        let line = raw.trim();
        if let Some(header) = line.strip_prefix("[").and_then(|l| l.strip_suffix("]")) {
            in_section = header == section;
            continue;
        }
        if line.is_empty() || line.starts_with('#') || !in_section {
            continue;
        }
        let (key, reason) = line
            .split_once(':')
            .unwrap_or_else(|| panic!("exemption line in [{section}] is not '<key> : <reason>': {line}"));
        let (key, reason) = (key.trim().to_string(), reason.trim().to_string());
        assert!(
            !reason.is_empty(),
            "exemption '{key}' in [{section}] has no reason; every exemption must say why it is exempt"
        );
        let previous = entries.insert(key.clone(), reason);
        assert!(previous.is_none(), "duplicate exemption '{key}' in [{section}]");
    }
    entries
}

fn exemptions_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("coverage-exemptions.txt")
}

/// Compare an observed gap set against its allowlist, failing in both
/// directions. Returns nothing; panics with the full delta on either failure.
fn assert_ratchet(observed_gaps: &BTreeMap<String, String>, allowed: &BTreeMap<String, String>, what: &str) {
    let observed_keys: BTreeSet<&String> = observed_gaps.keys().collect();
    let allowed_keys: BTreeSet<&String> = allowed.keys().collect();

    let regressions: Vec<String> = observed_keys
        .difference(&allowed_keys)
        .map(|key| format!("  {key} — {}", observed_gaps[*key]))
        .collect();
    let stale: Vec<String> = allowed_keys
        .difference(&observed_keys)
        .map(|key| format!("  {key} — listed as exempt because: {}", allowed[*key]))
        .collect();

    let mut failure = String::new();
    if !regressions.is_empty() {
        failure.push_str(&format!(
            "{what}: {} entr{} with no coverage and no exemption:\n{}\n\
             Close the gap, or add a line to {} under [{what}] giving the reason it \
             cannot be closed.\n",
            regressions.len(),
            if regressions.len() == 1 { "y" } else { "ies" },
            regressions.join("\n"),
            exemptions_path().display(),
        ));
    }
    if !stale.is_empty() {
        failure.push_str(&format!(
            "{what}: {} stale exemption{} — these are covered now, so the entr{} must be deleted \
             from {}:\n{}\n",
            stale.len(),
            if stale.len() == 1 { "" } else { "s" },
            if stale.len() == 1 { "y" } else { "ies" },
            exemptions_path().display(),
            stale.join("\n"),
        ));
    }
    assert!(failure.is_empty(), "\n{failure}");
}

/// #126/#134: every shipped backend manifest is exercised by an integration
/// project, or is explicitly and justifiably exempt.
#[test]
fn every_backend_manifest_has_an_integration_project() {
    let shipped = shipped_manifests();
    let covered = covered_manifests();

    // Coverage that resolves to no shipped manifest is impossible by
    // construction (the resolver returned it), but assert it rather than
    // assume it: it is the invariant that makes the gap set below meaningful.
    for pair in covered.keys() {
        assert!(
            shipped.contains_key(pair),
            "resolver produced manifest ({}, {}) that is not a file under crates/scythe-codegen/manifests",
            pair.0,
            pair.1
        );
    }

    let gaps: BTreeMap<String, String> = shipped
        .iter()
        .filter(|(pair, _)| !covered.contains_key(*pair))
        .map(|((name, engine), file)| (format!("{name}|{engine}"), format!("manifest {file}")))
        .collect();

    assert_ratchet(&gaps, &load_exemptions(&exemptions_path(), "manifests"), "manifests");
}

/// #135: every integration project is runnable locally, i.e. has a
/// `test:<project>` target in `integration_tests/Taskfile.yaml`. A project only
/// reachable by hand-assembling a command out of the CI workflow is a project
/// nobody debugs.
#[test]
fn every_integration_project_has_a_taskfile_target() {
    let taskfile = repo_root().join("integration_tests/Taskfile.yaml");
    let contents = read_to_string(&taskfile);

    let gaps: BTreeMap<String, String> = integration_projects()
        .into_iter()
        .filter(|(project, _)| !contents.contains(&format!("\n  test:{project}:")))
        .map(|(project, _)| (project, format!("no 'test:*' target in {}", taskfile.display())))
        .collect();

    assert_ratchet(
        &gaps,
        &load_exemptions(&exemptions_path(), "taskfile-targets"),
        "taskfile-targets",
    );
}

/// #118/#134: every integration project is actually executed by a job in the
/// integration workflow.
///
/// Without this, the manifest check above is satisfiable by committing a
/// project nothing runs — generated output that the freshness check inspects
/// and no driver ever touches. That is precisely how the csharp-snowflake
/// parameter-binding bug survived from v0.6.0 to 0.14.0, so closing the
/// manifest gap and leaving this one open would just relocate the defect.
///
/// The workflow is matched on `working-directory: integration_tests/<project>`,
/// which is how every step there names its project.
#[test]
fn every_integration_project_runs_in_ci() {
    let workflow = repo_root().join(".github/workflows/integration.yml");
    let contents = read_to_string(&workflow);

    let gaps: BTreeMap<String, String> = integration_projects()
        .into_iter()
        .filter(|(project, _)| !contents.contains(&format!("working-directory: integration_tests/{project}\n")))
        .map(|(project, _)| (project, format!("no step in {}", workflow.display())))
        .collect();

    assert_ratchet(&gaps, &load_exemptions(&exemptions_path(), "ci-steps"), "ci-steps");
}

/// Every generated harness executes the shared schema by splitting it on `";"` and running one
/// fragment at a time (`elixir.exs.jinja`, `kotlin.kt.jinja`, `java.java.jinja`, `php.php.jinja`,
/// `ruby.rb.jinja`, `typescript.ts.jinja`, `python.py.jinja`, `csharp.cs.jinja` all do this). That
/// split is not SQL-aware: a `;` inside a `--` comment ends the fragment there, and the rest of
/// the comment line — no longer preceded by its `--` — becomes the head of the next fragment and
/// is sent to the server as bare SQL.
///
/// This is not hypothetical. `sql/pg/schema.sql` carried the comment "...under compile-only
/// coverage; this one is exercised live...", and `elixir-postgrex` failed with
/// `ERROR 42601 (syntax_error) syntax error at or near "this"` — the fragment began `this one`.
/// Only elixir surfaced it because the postgres job fails fast and the harnesses ahead of it in
/// the job happen not to split that schema, so the same latent break sat behind them.
///
/// Splitting SQL naively is the real defect and it is tracked separately; until the harnesses
/// parse properly, this keeps the fixture side of the contract enforced rather than relying on
/// whoever edits a schema comment remembering an invariant nothing states.
#[test]
fn schema_sql_comments_contain_no_semicolon() {
    let sql_root = repo_root().join("integration_tests/sql");
    let mut offenders: Vec<String> = Vec::new();
    let mut files_scanned = 0usize;

    let mut stack = vec![sql_root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("sql") {
                continue;
            }
            files_scanned += 1;
            for (index, line) in read_to_string(&path).lines().enumerate() {
                // ~keep Only a whole-line `--` comment can strand its tail as SQL. A trailing comment
                // after a statement is preceded by that statement's own text, and a `;` inside a
                // string literal is a genuine statement terminator question this does not touch.
                let trimmed = line.trim_start();
                if trimmed.starts_with("--") && trimmed.contains(';') {
                    offenders.push(format!(
                        "  {}:{}: {}",
                        path.strip_prefix(repo_root()).unwrap_or(&path).display(),
                        index + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    // ~keep A zero-file scan would make this pass vacuously — the same shape the `[ci-steps]` ratchet
    // guards against. `integration_tests/sql` has never been empty.
    assert!(
        files_scanned > 0,
        "scanned no .sql files under {}; this check would pass without inspecting anything",
        sql_root.display()
    );
    assert!(
        offenders.is_empty(),
        "a `;` inside a whole-line SQL comment breaks every harness that splits the schema on \
         \";\" -- rewrite the comment without it:\n{}",
        offenders.join("\n")
    );
}
