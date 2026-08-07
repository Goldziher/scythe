//! The divergence registry: `testing_data/nullability_live/DIVERGENCES.toml`.
//!
//! This is a narrow escape hatch for one specific, legitimate situation:
//! the analyzer is more pessimistic than reality on some engine (it marked
//! a column nullable, but no live run has ever demonstrated a NULL for it
//! there), and that's accepted rather than fixed. It can never explain away
//! an A2 soundness failure -- there is only one legal [`DivergenceKind`],
//! and it is not capable of expressing that suppression, so a mistaken or
//! malicious entry cannot silence a real crash-on-decode bug.
//!
//! A registered divergence that stops reproducing fails the build: fixing
//! the underlying bug thus forces deleting the entry that excused it.

use std::path::Path;

use serde::Deserialize;

use crate::fixture::Engine;
use crate::verdict::{Failure, Verdict};

/// Hard cap on the number of entries the registry may hold, enforced by a
/// test in this crate. Kept low on purpose -- every entry is a known gap
/// between the analyzer and one engine's behavior, and a growing list is a
/// growing liability, not a convenience. This constant may only ever be
/// lowered; raising it is a deliberate policy change and must be visible in
/// code review (see `committed_registry_cap_may_only_ever_be_lowered`
/// below, which pins the current value).
pub const MAX_ENTRIES: usize = 25;

/// The only legal reason to register a divergence: the analyzer is more
/// pessimistic than the live engine actually is. There is no variant for
/// "the analyzer is more optimistic than reality" (an A2 failure) -- that
/// case has no legal `kind`, so the loader rejects it by construction: an
/// unrecognized `kind` string fails to deserialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceKind {
    AnalyzerOverPessimistic,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DivergenceEntry {
    pub fixture: String,
    pub engine: String,
    pub column: String,
    pub kind: DivergenceKind,
    /// URL to the tracking issue -- required so every accepted gap has an
    /// owner and a place to close it out.
    pub issue: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DivergenceFile {
    #[serde(default, rename = "entry")]
    entries: Vec<DivergenceEntry>,
}

#[derive(Debug, thiserror::Error)]
pub enum DivergenceError {
    #[error("reading divergence registry {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing divergence registry {path}: {source}")]
    Parse {
        path: std::path::PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("divergence entry for {fixture}/{engine}/{column} has a non-URL issue: {issue:?}")]
    InvalidIssueUrl {
        fixture: String,
        engine: String,
        column: String,
        issue: String,
    },
    #[error("divergence registry has {count} entries, exceeding the cap of {max}")]
    TooManyEntries { count: usize, max: usize },
    #[error("divergence for {fixture}/{engine}/{column} no longer reproduces; remove it")]
    Stale {
        fixture: String,
        engine: String,
        column: String,
    },
}

/// Load and validate the registry at `path`: every issue must look like a
/// URL, and the entry count must stay under [`MAX_ENTRIES`]. Does not check
/// staleness -- see [`check_staleness`], which needs a set of verdicts to
/// check against.
pub fn load(path: &Path) -> Result<Vec<DivergenceEntry>, DivergenceError> {
    let contents = std::fs::read_to_string(path).map_err(|source| DivergenceError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file: DivergenceFile = toml::from_str(&contents).map_err(|source| DivergenceError::Parse {
        path: path.to_path_buf(),
        source,
    })?;

    if file.entries.len() > MAX_ENTRIES {
        return Err(DivergenceError::TooManyEntries {
            count: file.entries.len(),
            max: MAX_ENTRIES,
        });
    }

    for entry in &file.entries {
        if !(entry.issue.starts_with("https://") || entry.issue.starts_with("http://")) {
            return Err(DivergenceError::InvalidIssueUrl {
                fixture: entry.fixture.clone(),
                engine: entry.engine.clone(),
                column: entry.column.clone(),
                issue: entry.issue.clone(),
            });
        }
    }

    Ok(file.entries)
}

/// Whether `kind` is capable of suppressing `failure` for `column` at all.
///
/// Matches `(kind, failure)` exhaustively, with no wildcard arm: adding a
/// new [`DivergenceKind`] variant or a new [`Failure`] variant makes this
/// non-exhaustive and fails to compile until every combination has an
/// explicit, reviewed answer. That is what keeps "no divergence can ever
/// suppress an A2 soundness failure" true as the type grows, rather than
/// true only because there happened to be one variant when it was written.
fn kind_suppresses(kind: DivergenceKind, failure: &Failure, column: &str) -> bool {
    match (kind, failure) {
        (DivergenceKind::AnalyzerOverPessimistic, Failure::VacuousNullable { column: c }) => c == column,
        (DivergenceKind::AnalyzerOverPessimistic, Failure::FidelityMismatch { .. }) => false,
        (DivergenceKind::AnalyzerOverPessimistic, Failure::UnsoundNullability { .. }) => false,
        (DivergenceKind::AnalyzerOverPessimistic, Failure::JoinGroupIncoherent { .. }) => false,
    }
}

/// Suppress only the specific `Failure::VacuousNullable` (A3) failures that
/// `entries` registers for a matching (fixture, engine, column). Every
/// other failure variant -- including `Failure::UnsoundNullability` (A2) --
/// passes through untouched; see [`kind_suppresses`].
///
/// Must be called with the *raw* verdicts from [`crate::verdict::evaluate`],
/// before staleness has been checked -- see [`reconcile`], which gets the
/// order right structurally rather than relying on callers to remember it.
pub fn apply(entries: &[DivergenceEntry], verdicts: Vec<Verdict>) -> Vec<Verdict> {
    verdicts
        .into_iter()
        .map(|verdict| {
            let failures = verdict
                .failures
                .into_iter()
                .filter(|failure| !is_registered(entries, &verdict.fixture, verdict.engine.as_str(), failure))
                .collect();
            Verdict { failures, ..verdict }
        })
        .collect()
}

fn is_registered(entries: &[DivergenceEntry], fixture: &str, engine: &str, failure: &Failure) -> bool {
    entries.iter().any(|entry| {
        entry.fixture == fixture && entry.engine == engine && kind_suppresses(entry.kind, failure, &entry.column)
    })
}

/// Fails the build if a registered divergence no longer reproduces in
/// `verdicts`, for any engine actually present in `engines_run`.
///
/// `verdicts` must be the *raw* output of [`crate::verdict::evaluate`] --
/// i.e. **not** yet passed through [`apply`]. `apply` removes exactly the
/// failures this function looks for, so calling this after `apply` makes
/// every entry look stale regardless of whether it still reproduces. Use
/// [`reconcile`] to get this ordering for free instead of relying on
/// callers to remember it.
///
/// An entry for an engine that isn't in `engines_run` at all (a partial
/// run, e.g. only Postgres was dialed this time) is skipped rather than
/// treated as stale: "we didn't check" and "it no longer reproduces" are
/// different facts, and conflating them would make every entry for an
/// untested engine fail on every partial run.
pub fn check_staleness(
    entries: &[DivergenceEntry],
    verdicts: &[Verdict],
    engines_run: &[Engine],
) -> Result<(), DivergenceError> {
    for entry in entries {
        let entry_engine_was_run = engines_run.iter().any(|e| e.as_str() == entry.engine);
        if !entry_engine_was_run {
            continue;
        }

        let still_reproduces = verdicts.iter().any(|verdict| {
            verdict.fixture == entry.fixture
                && verdict.engine.as_str() == entry.engine
                && verdict
                    .failures
                    .iter()
                    .any(|f| kind_suppresses(entry.kind, f, &entry.column))
        });
        if !still_reproduces {
            return Err(DivergenceError::Stale {
                fixture: entry.fixture.clone(),
                engine: entry.engine.clone(),
                column: entry.column.clone(),
            });
        }
    }
    Ok(())
}

/// The recommended entrypoint for applying the divergence registry to a
/// full evaluation run: checks staleness against the raw (pre-suppression)
/// `verdicts` first, then applies suppression.
///
/// Combining these into one function makes the correct order -- staleness
/// before suppression -- structural rather than a convention callers have
/// to remember: see [`check_staleness`] for why calling it after [`apply`]
/// makes every entry look stale.
pub fn reconcile(
    entries: &[DivergenceEntry],
    verdicts: Vec<Verdict>,
    engines_run: &[Engine],
) -> Result<Vec<Verdict>, DivergenceError> {
    check_staleness(entries, &verdicts, engines_run)?;
    Ok(apply(entries, verdicts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Engine;

    fn entry(fixture: &str, engine: &str, column: &str) -> DivergenceEntry {
        DivergenceEntry {
            fixture: fixture.to_string(),
            engine: engine.to_string(),
            column: column.to_string(),
            kind: DivergenceKind::AnalyzerOverPessimistic,
            issue: "https://github.com/Goldziher/scythe/issues/1".to_string(),
            reason: "test".to_string(),
        }
    }

    fn verdict(fixture: &str, engine: Engine, failures: Vec<Failure>) -> Verdict {
        Verdict {
            fixture: fixture.to_string(),
            engine,
            failures,
        }
    }

    #[test]
    fn apply_suppresses_a_matching_vacuity_failure() {
        let entries = vec![entry("live_x", "oracle", "notes")];
        let verdicts = vec![verdict(
            "live_x",
            Engine::Oracle,
            vec![Failure::VacuousNullable {
                column: "notes".to_string(),
            }],
        )];
        let result = apply(&entries, verdicts);
        assert!(result[0].is_pass(), "{:?}", result[0].failures);
    }

    #[test]
    fn apply_leaves_unmatched_vacuity_failures_alone() {
        let entries = vec![entry("live_x", "oracle", "notes")];
        let verdicts = vec![verdict(
            "live_x",
            Engine::Postgresql,
            vec![Failure::VacuousNullable {
                column: "notes".to_string(),
            }],
        )];
        let result = apply(&entries, verdicts);
        assert_eq!(result[0].failures.len(), 1);
    }

    #[test]
    fn apply_never_suppresses_unsound_nullability() {
        let entries = vec![entry("live_x", "oracle", "id")];
        let verdicts = vec![verdict(
            "live_x",
            Engine::Oracle,
            vec![Failure::UnsoundNullability {
                column: "id".to_string(),
                row: 0,
            }],
        )];
        let result = apply(&entries, verdicts);
        assert_eq!(result[0].failures.len(), 1, "A2 must never be suppressible");
    }

    #[test]
    fn apply_never_suppresses_fidelity_mismatch() {
        let entries = vec![entry("live_x", "oracle", "id")];
        let verdicts = vec![verdict(
            "live_x",
            Engine::Oracle,
            vec![Failure::FidelityMismatch {
                column: "id".to_string(),
                analyzed_nullable: true,
                generated_nullable: false,
            }],
        )];
        let result = apply(&entries, verdicts);
        assert_eq!(result[0].failures.len(), 1, "A1 must never be suppressible");
    }

    #[test]
    fn apply_never_suppresses_join_group_incoherent() {
        let entries = vec![entry("live_x", "oracle", "id")];
        let verdicts = vec![verdict(
            "live_x",
            Engine::Oracle,
            vec![Failure::JoinGroupIncoherent {
                group: "o".to_string(),
                row: 0,
                columns: vec!["id".to_string()],
            }],
        )];
        let result = apply(&entries, verdicts);
        assert_eq!(result[0].failures.len(), 1, "A4 must never be suppressible");
    }

    #[test]
    fn divergence_kind_match_is_exhaustive() {
        // If `DivergenceKind` gains a variant, this match becomes
        // non-exhaustive and fails to compile -- forcing `kind_suppresses`
        // to be updated (for every `Failure` variant) before this crate
        // builds again.
        let kind = DivergenceKind::AnalyzerOverPessimistic;
        match kind {
            DivergenceKind::AnalyzerOverPessimistic => {}
        }
    }

    #[test]
    fn loader_rejects_a_kind_other_than_analyzer_over_pessimistic() {
        let toml = r#"
[[entry]]
fixture = "live_x"
engine = "oracle"
column = "notes"
kind = "engine_more_permissive"
issue = "https://github.com/Goldziher/scythe/issues/1"
reason = "test"
"#;
        let result: Result<DivergenceFile, _> = toml::from_str(toml);
        assert!(result.is_err(), "unknown divergence kind must fail to deserialize");
    }

    #[test]
    fn loader_rejects_an_unknown_field() {
        let toml = r#"
[[entry]]
fixture = "live_x"
engine = "oracle"
column = "notes"
kind = "analyzer_over_pessimistic"
issue = "https://github.com/Goldziher/scythe/issues/1"
reason = "test"
suppresses_soundness = true
"#;
        let result: Result<DivergenceFile, _> = toml::from_str(toml);
        assert!(result.is_err(), "an unrecognized field must fail to deserialize");
    }

    #[test]
    fn loader_rejects_the_wrong_table_key_silently_yielding_zero_entries() {
        // `[[entries]]` (plural) used to silently deserialize to zero
        // entries under `#[serde(default, rename = "entry")]` with no
        // `deny_unknown_fields` -- every registered divergence would
        // evaporate without a diagnostic. It must now be a hard parse
        // error.
        let toml = r#"
[[entries]]
fixture = "live_x"
engine = "oracle"
column = "notes"
kind = "analyzer_over_pessimistic"
issue = "https://github.com/Goldziher/scythe/issues/1"
reason = "test"
"#;
        let result: Result<DivergenceFile, _> = toml::from_str(toml);
        assert!(
            result.is_err(),
            "the wrong table key must fail to deserialize, not silently yield zero entries"
        );
    }

    #[test]
    fn loader_rejects_a_non_url_issue() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("DIVERGENCES.toml");
        std::fs::write(
            &path,
            r#"
[[entry]]
fixture = "live_x"
engine = "oracle"
column = "notes"
kind = "analyzer_over_pessimistic"
issue = "not-a-url"
reason = "test"
"#,
        )
        .unwrap();
        let result = load(&path);
        assert!(matches!(result, Err(DivergenceError::InvalidIssueUrl { .. })));
    }

    #[test]
    fn loader_rejects_too_many_entries() {
        let mut toml = String::new();
        for i in 0..(MAX_ENTRIES + 1) {
            toml.push_str(&format!(
                "[[entry]]\nfixture = \"live_{i}\"\nengine = \"oracle\"\ncolumn = \"c\"\nkind = \"analyzer_over_pessimistic\"\nissue = \"https://github.com/Goldziher/scythe/issues/{i}\"\nreason = \"test\"\n\n"
            ));
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("DIVERGENCES.toml");
        std::fs::write(&path, toml).unwrap();
        let result = load(&path);
        assert!(matches!(result, Err(DivergenceError::TooManyEntries { .. })));
    }

    #[test]
    fn loader_parses_a_well_formed_entry_field_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("DIVERGENCES.toml");
        std::fs::write(
            &path,
            r#"
[[entry]]
fixture = "live_right_side_not_null_becomes_nullable"
engine = "oracle"
column = "notes"
kind = "analyzer_over_pessimistic"
issue = "https://github.com/Goldziher/scythe/issues/42"
reason = "Oracle never returns NULL for this synthetic column in any live run"
"#,
        )
        .unwrap();
        let entries = load(&path).expect("well-formed entry must load");
        assert_eq!(entries.len(), 1);
        let parsed = &entries[0];
        assert_eq!(parsed.fixture, "live_right_side_not_null_becomes_nullable");
        assert_eq!(parsed.engine, "oracle");
        assert_eq!(parsed.column, "notes");
        assert_eq!(parsed.kind, DivergenceKind::AnalyzerOverPessimistic);
        assert_eq!(parsed.issue, "https://github.com/Goldziher/scythe/issues/42");
        assert_eq!(
            parsed.reason,
            "Oracle never returns NULL for this synthetic column in any live run"
        );
    }

    #[test]
    fn check_staleness_fails_when_a_registered_divergence_no_longer_reproduces() {
        let entries = vec![entry("live_x", "oracle", "notes")];
        let verdicts = vec![verdict("live_x", Engine::Oracle, vec![])];
        let result = check_staleness(&entries, &verdicts, &[Engine::Oracle]);
        assert!(matches!(result, Err(DivergenceError::Stale { .. })));
    }

    #[test]
    fn check_staleness_passes_when_the_divergence_still_reproduces() {
        let entries = vec![entry("live_x", "oracle", "notes")];
        let verdicts = vec![verdict(
            "live_x",
            Engine::Oracle,
            vec![Failure::VacuousNullable {
                column: "notes".to_string(),
            }],
        )];
        let result = check_staleness(&entries, &verdicts, &[Engine::Oracle]);
        assert!(result.is_ok());
    }

    #[test]
    fn check_staleness_skips_entries_for_engines_that_were_not_run() {
        // A partial run (only Postgres this time) must not mark every
        // Oracle-scoped entry stale just because Oracle wasn't dialed.
        let entries = vec![entry("live_x", "oracle", "notes")];
        let verdicts = vec![verdict("live_x", Engine::Postgresql, vec![])];
        let result = check_staleness(&entries, &verdicts, &[Engine::Postgresql]);
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn check_staleness_is_order_independent_of_apply_when_used_correctly() {
        // Calling check_staleness on the *raw* verdicts (before apply) must
        // succeed for an entry that still reproduces, regardless of what
        // apply would later do to those same verdicts.
        let entries = vec![entry("live_x", "oracle", "notes")];
        let raw_verdicts = vec![verdict(
            "live_x",
            Engine::Oracle,
            vec![Failure::VacuousNullable {
                column: "notes".to_string(),
            }],
        )];
        assert!(check_staleness(&entries, &raw_verdicts, &[Engine::Oracle]).is_ok());

        // The same verdicts, post-apply, would (incorrectly, if reused)
        // report every entry as stale -- demonstrating why `reconcile`
        // exists instead of leaving the ordering to convention.
        let applied = apply(&entries, raw_verdicts);
        assert!(applied[0].is_pass());
        assert!(check_staleness(&entries, &applied, &[Engine::Oracle]).is_err());
    }

    #[test]
    fn reconcile_checks_staleness_before_suppressing_regardless_of_call_site_order() {
        let entries = vec![entry("live_x", "oracle", "notes")];
        let verdicts = vec![verdict(
            "live_x",
            Engine::Oracle,
            vec![Failure::VacuousNullable {
                column: "notes".to_string(),
            }],
        )];
        let result = reconcile(&entries, verdicts, &[Engine::Oracle]).expect("a still-reproducing entry must pass");
        assert!(result[0].is_pass(), "{:?}", result[0].failures);
    }

    #[test]
    fn reconcile_fails_for_a_stale_entry_instead_of_silently_suppressing_nothing() {
        let entries = vec![entry("live_x", "oracle", "notes")];
        let verdicts = vec![verdict("live_x", Engine::Oracle, vec![])];
        let result = reconcile(&entries, verdicts, &[Engine::Oracle]);
        assert!(matches!(result, Err(DivergenceError::Stale { .. })));
    }

    #[test]
    fn committed_registry_stays_under_the_cap() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing_data/nullability_live/DIVERGENCES.toml");
        let entries = load(&path).expect("committed DIVERGENCES.toml must load cleanly");
        assert!(entries.len() <= MAX_ENTRIES);
        for entry in &entries {
            assert!(!entry.issue.is_empty());
        }
    }

    #[test]
    fn max_entries_cap_may_only_ever_be_lowered() {
        // A change-detector by design: MAX_ENTRIES must never silently grow.
        // Raising it is a deliberate policy decision that belongs in code
        // review, not an incidental side effect of adding entries -- lower
        // this assertion (and the constant) freely, but raising either
        // should prompt a second look at why the registry is growing.
        assert_eq!(
            MAX_ENTRIES, 25,
            "lower this cap freely; don't raise it without a deliberate policy decision"
        );
    }
}
