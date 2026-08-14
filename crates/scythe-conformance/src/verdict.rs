//! The four-assertion comparison at the heart of this crate.
//!
//! Per (fixture, engine, column), three facts are compared:
//!
//! - **A** -- the analyzer's `AnalyzedColumn::nullable`
//! - **G** -- whether the generated code's `ResolvedColumn::full_type` is
//!   actually rendered as optional under the backend manifest (see
//!   [`ColumnFacts::from_analyzed_and_generated`] -- this is *not* the same
//!   as `ResolvedColumn::nullable`, which is copied verbatim from `A` and so
//!   can never disagree with it)
//! - **E** -- the engine's observed per-row nullness
//!
//! Every function here is pure and operates on already-computed facts --
//! there is no database access in this module, which is what keeps it
//! ~100% unit-testable.

use ahash::AHashMap;
use scythe_backend::manifest::BackendManifest;
use scythe_codegen::backend_trait::ResolvedColumn;
use scythe_core::analyzer::AnalyzedColumn;

use crate::fixture::Engine;

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

/// Per-(fixture, engine, column) facts needed to evaluate the four
/// assertions. Build from real analyzer/codegen output via
/// [`ColumnFacts::from_analyzed_and_generated`], or directly in tests.
#[derive(Debug, Clone)]
pub struct ColumnFacts {
    pub column: String,
    /// A: the analyzer's `AnalyzedColumn::nullable`.
    pub analyzed_nullable: bool,
    /// G: whether the generated code actually *renders* this column as
    /// optional. Built by [`ColumnFacts::from_analyzed_and_generated`] by
    /// parsing `ResolvedColumn::full_type` against the backend manifest's
    /// nullable-wrapping pattern -- deliberately never copied from
    /// `ResolvedColumn::nullable`, which mirrors `AnalyzedColumn::nullable`
    /// by construction (see `scythe_codegen::resolve::resolve_columns`) and
    /// so can never disagree with `analyzed_nullable`. Comparing against
    /// that copy would make [`check_fidelity`] unable to fail for any facts
    /// built from the real pipeline.
    pub generated_nullable: bool,
    pub join_group: Option<String>,
    pub nullable_before_join: bool,
    /// E: one entry per observed row (across all of a fixture's runs, in a
    /// fixed order), `true` when the engine returned NULL for this column
    /// in that row.
    pub observed_nulls: Vec<bool>,
}

impl ColumnFacts {
    /// Build facts from real analyzer/codegen output. `analyzed` and
    /// `generated` must describe the same column --
    /// `scythe_codegen::resolve::resolve_columns` preserves `join_group` and
    /// `nullable_before_join` unchanged from the `AnalyzedColumn` it
    /// resolves, so both always agree on those two fields by construction.
    ///
    /// `generated_nullable` is *not* read from `generated.nullable` (see the
    /// field doc on [`ColumnFacts::generated_nullable`] for why): it is
    /// parsed from `generated.full_type` against `manifest`'s nullable
    /// container pattern via
    /// [`scythe_backend::types::parse_rendered_nullable`]. This can fail if
    /// `full_type` matches neither the base nor the wrapped form under
    /// `manifest` -- always a bug (a manifest missing its `nullable`
    /// pattern, or a `ResolvedColumn` that didn't actually come from this
    /// manifest).
    pub fn from_analyzed_and_generated(
        analyzed: &AnalyzedColumn,
        generated: &ResolvedColumn,
        manifest: &BackendManifest,
        observed_nulls: Vec<bool>,
    ) -> Result<Self, ColumnFactsError> {
        let generated_nullable =
            scythe_backend::types::parse_rendered_nullable(&generated.lang_type, &generated.full_type, manifest)
                .map_err(|source| ColumnFactsError::RenderedNullability {
                    column: generated.name.clone(),
                    source,
                })?;
        Ok(Self {
            column: analyzed.name.clone(),
            analyzed_nullable: analyzed.nullable,
            generated_nullable,
            join_group: analyzed.join_group.clone(),
            nullable_before_join: analyzed.nullable_before_join,
            observed_nulls,
        })
    }
}

/// Error building [`ColumnFacts`] from real analyzer/codegen output.
#[derive(Debug, thiserror::Error)]
pub enum ColumnFactsError {
    #[error("column {column:?}: could not determine rendered nullability: {source}")]
    RenderedNullability {
        column: String,
        #[source]
        source: scythe_backend::BackendError,
    },
}

/// All column facts for one (fixture, engine) pair, aligned by row index:
/// `columns()[i].observed_nulls[r]` and `columns()[j].observed_nulls[r]`
/// refer to the same observed row `r`. Constructed only via
/// [`FixtureEngineFacts::new`], which enforces that invariant, so a
/// short-or-long `observed_nulls` vector can never reach [`evaluate`].
#[derive(Debug, Clone)]
pub struct FixtureEngineFacts {
    fixture: String,
    engine: Engine,
    columns: Vec<ColumnFacts>,
    row_count: usize,
}

/// `column` declares a number of observed rows that doesn't match
/// `row_count` for the rest of the [`FixtureEngineFacts`] it was built into.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("column {column:?} has {actual} observed row(s) but this fixture/engine declares {expected}")]
pub struct RowCountMismatch {
    pub column: String,
    pub expected: usize,
    pub actual: usize,
}

impl FixtureEngineFacts {
    /// Validating constructor: every column's `observed_nulls` must have
    /// exactly `row_count` entries. Before this constructor existed,
    /// [`check_join_group_coherence`] filled short vectors in with
    /// `unwrap_or(false)` -- silently treating "we never observed this row
    /// for this column" as "observed non-null", which both fabricates
    /// coherence and masks the underlying data bug. Going through `new` is
    /// the only way to build a [`FixtureEngineFacts`], so that case is now
    /// unreachable by construction rather than merely untested.
    pub fn new(
        fixture: impl Into<String>,
        engine: Engine,
        columns: Vec<ColumnFacts>,
        row_count: usize,
    ) -> Result<Self, RowCountMismatch> {
        for column in &columns {
            if column.observed_nulls.len() != row_count {
                return Err(RowCountMismatch {
                    column: column.column.clone(),
                    expected: row_count,
                    actual: column.observed_nulls.len(),
                });
            }
        }
        Ok(Self {
            fixture: fixture.into(),
            engine,
            columns,
            row_count,
        })
    }

    pub fn fixture(&self) -> &str {
        &self.fixture
    }

    pub fn engine(&self) -> Engine {
        self.engine
    }

    pub fn columns(&self) -> &[ColumnFacts] {
        &self.columns
    }

    pub fn row_count(&self) -> usize {
        self.row_count
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

/// A single assertion failure. Every variant names the column (and, for
/// A2/A4, the row) it was found on so a report can point straight at it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// A1: the analyzer's inference and the generated code's *rendered*
    /// nullability (see [`ColumnFacts::generated_nullable`]) disagree for
    /// the same column.
    FidelityMismatch {
        column: String,
        analyzed_nullable: bool,
        generated_nullable: bool,
    },
    /// A2 soundness: the engine produced NULL in a column the generated
    /// code renders as non-optional -- it would non-optionally decode a
    /// NULL and crash. Keyed on `generated_nullable`, not
    /// `analyzed_nullable`: the crash is a property of what code actually
    /// got generated, so if A1 has already failed (analyzer and codegen
    /// disagree), A2 must still reflect the real risk, not the analyzer's
    /// opinion of it. Always a hard failure; nothing in this crate can
    /// suppress it (see [`crate::divergence`]).
    UnsoundNullability { column: String, row: usize },
    /// A3 anti-vacuity: the analyzer called a column nullable, but no
    /// observed row across any run demonstrates a NULL for it. Without this
    /// check the suite is satisfied by marking every column nullable.
    VacuousNullable { column: String },
    /// A4 join-group coherence: columns sharing a `join_group` with
    /// `nullable_before_join == false` must be null together in no-match
    /// rows and non-null together in matched rows.
    JoinGroupIncoherent {
        group: String,
        row: usize,
        columns: Vec<String>,
    },
}

/// The outcome of evaluating all four assertions for one (fixture, engine)
/// pair.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub fixture: String,
    pub engine: Engine,
    pub failures: Vec<Failure>,
}

impl Verdict {
    pub fn is_pass(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Evaluate all four assertions for `facts`, returning one [`Verdict`] with
/// every failure found -- nothing here short-circuits on the first failure,
/// so a report shows the full picture in one pass.
pub fn evaluate(facts: &FixtureEngineFacts) -> Verdict {
    let mut failures = Vec::new();

    for column in facts.columns() {
        failures.extend(check_fidelity(column));
        failures.extend(check_soundness(column));
        failures.extend(check_anti_vacuity(column));
    }
    failures.extend(check_join_group_coherence(facts.columns(), facts.row_count()));

    Verdict {
        fixture: facts.fixture().to_string(),
        engine: facts.engine(),
        failures,
    }
}

/// A1: `A == G`.
fn check_fidelity(column: &ColumnFacts) -> Option<Failure> {
    if column.analyzed_nullable == column.generated_nullable {
        return None;
    }
    Some(Failure::FidelityMismatch {
        column: column.column.clone(),
        analyzed_nullable: column.analyzed_nullable,
        generated_nullable: column.generated_nullable,
    })
}

/// A2: `engine_returned_null(i) => G`, for every observed row. See
/// [`Failure::UnsoundNullability`] for why this keys on `generated_nullable`
/// rather than `analyzed_nullable`.
fn check_soundness(column: &ColumnFacts) -> Vec<Failure> {
    column
        .observed_nulls
        .iter()
        .enumerate()
        .filter(|&(_, &is_null)| is_null && !column.generated_nullable)
        .map(|(row, _)| Failure::UnsoundNullability {
            column: column.column.clone(),
            row,
        })
        .collect()
}

/// A3: `A(i) => some run demonstrates a NULL for i`.
fn check_anti_vacuity(column: &ColumnFacts) -> Option<Failure> {
    if !column.analyzed_nullable {
        return None;
    }
    if column.observed_nulls.iter().any(|&is_null| is_null) {
        return None;
    }
    Some(Failure::VacuousNullable {
        column: column.column.clone(),
    })
}

/// A4: columns sharing a `join_group` with `nullable_before_join == false`
/// must be null together in no-match rows and non-null together in matched
/// rows. Groups are visited in first-occurrence order and rows in index
/// order, so failures come back in a stable, reproducible sequence.
///
/// `row_count` and every column's `observed_nulls` length are guaranteed
/// equal by [`FixtureEngineFacts::new`], so indexing `observed_nulls[row]`
/// directly (rather than falling back to a default on a short vector) can
/// never panic and never fabricates a "non-null" that was never observed.
fn check_join_group_coherence(columns: &[ColumnFacts], row_count: usize) -> Vec<Failure> {
    let mut group_order: Vec<String> = Vec::new();
    let mut groups: AHashMap<String, Vec<&ColumnFacts>> = AHashMap::new();

    for column in columns {
        let Some(alias) = &column.join_group else { continue };
        if column.nullable_before_join {
            continue;
        }
        if !groups.contains_key(alias) {
            group_order.push(alias.clone());
        }
        groups.entry(alias.clone()).or_default().push(column);
    }

    let mut failures = Vec::new();
    for alias in group_order {
        let members = &groups[&alias];
        if members.len() < 2 {
            continue;
        }
        for row in 0..row_count {
            let nulls: Vec<bool> = members.iter().map(|c| c.observed_nulls[row]).collect();
            let all_null = nulls.iter().all(|&n| n);
            let none_null = nulls.iter().all(|&n| !n);
            if !all_null && !none_null {
                failures.push(Failure::JoinGroupIncoherent {
                    group: alias.clone(),
                    row,
                    columns: members.iter().map(|c| c.column.clone()).collect(),
                });
            }
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use scythe_core::analyzer::AnalyzedColumn;

    fn column(name: &str, analyzed: bool, generated: bool, observed_nulls: Vec<bool>) -> ColumnFacts {
        ColumnFacts {
            column: name.to_string(),
            analyzed_nullable: analyzed,
            generated_nullable: generated,
            join_group: None,
            nullable_before_join: false,
            observed_nulls,
        }
    }

    fn facts(columns: Vec<ColumnFacts>, row_count: usize) -> FixtureEngineFacts {
        FixtureEngineFacts::new("live_test", Engine::Postgresql, columns, row_count)
            .expect("test fixtures must have consistent observed_nulls lengths")
    }

    // -- A1 fidelity ---------------------------------------------------

    #[test]
    fn a1_passes_when_analyzer_and_codegen_agree() {
        let v = evaluate(&facts(vec![column("total", true, true, vec![true])], 1));
        assert!(v.is_pass(), "{:?}", v.failures);
    }

    #[test]
    fn a1_fires_when_analyzer_and_codegen_disagree() {
        // ~keep This input also fires A2: `generated_nullable == false` and the
        // column was observed NULL, which is a real soundness risk (see
        // `check_soundness`'s doc comment on why A2 keys on the generated
        // side, not the analyzed side) -- a fidelity mismatch that
        // under-renders nullability is caught by both assertions, not just
        // one.
        let v = evaluate(&facts(vec![column("total", true, false, vec![true])], 1));
        assert_eq!(
            v.failures,
            vec![
                Failure::FidelityMismatch {
                    column: "total".to_string(),
                    analyzed_nullable: true,
                    generated_nullable: false
                },
                Failure::UnsoundNullability {
                    column: "total".to_string(),
                    row: 0
                },
            ]
        );
    }

    // -- A1 fidelity through the real analyzer/codegen pipeline ---------

    fn test_manifest() -> BackendManifest {
        let toml_str = r#"
[backend]
name = "test-backend"
language = "rust"
file_extension = "rs"
engine = "postgresql"

[types.scalars]
int32 = "i32"

[types.containers]
nullable = "Option<{T}>"

[naming]
struct_case = "PascalCase"
fn_case = "snake_case"
enum_variant_case = "PascalCase"
row_suffix = "Row"
"#;
        toml::from_str(toml_str).expect("inline test manifest must parse")
    }

    fn analyzed_column(name: &str, nullable: bool) -> AnalyzedColumn {
        AnalyzedColumn {
            name: name.to_string(),
            neutral_type: "int32".to_string(),
            nullable,
            ..Default::default()
        }
    }

    #[test]
    fn from_analyzed_and_generated_agrees_through_the_real_resolve_pipeline() {
        let manifest = test_manifest();
        let analyzed = vec![analyzed_column("total", true)];
        let resolved = scythe_codegen::resolve::resolve_columns(&analyzed, &manifest, &[])
            .expect("resolve_columns must succeed for a well-formed manifest");

        let facts = ColumnFacts::from_analyzed_and_generated(&analyzed[0], &resolved[0], &manifest, vec![true])
            .expect("a well-formed manifest must yield a parseable rendering");

        assert!(facts.analyzed_nullable);
        assert!(facts.generated_nullable);
        let v = evaluate(&facts_from(vec![facts], 1));
        assert!(v.is_pass(), "{:?}", v.failures);
    }

    #[test]
    fn from_analyzed_and_generated_catches_a_manifest_that_cannot_render_nullable() {
        // ~keep A backend manifest whose "nullable" pattern is the identity
        // mapping can never render `Option<T>` -- this is the realistic A1
        // bug this crate exists to catch: the *manifest*, not the analyzer,
        // is broken. Before this fix, `generated_nullable` copied
        // `analyzed.nullable` verbatim (through
        // `ResolvedColumn::nullable`) and could never disagree with it, so
        // this exact scenario was unreachable via the real pipeline.
        let mut manifest = test_manifest();
        manifest
            .types
            .containers
            .insert("nullable".to_string(), "{T}".to_string());
        let analyzed = vec![analyzed_column("total", true)];
        let resolved = scythe_codegen::resolve::resolve_columns(&analyzed, &manifest, &[])
            .expect("resolve_columns must still succeed -- the bug is semantic, not a parse failure");

        let facts = ColumnFacts::from_analyzed_and_generated(&analyzed[0], &resolved[0], &manifest, vec![true])
            .expect("full_type still parses -- it just parses as non-optional");

        assert!(facts.analyzed_nullable);
        assert!(
            !facts.generated_nullable,
            "a manifest that can't render Option<T> must not be reported as nullable"
        );

        let v = evaluate(&facts_from(vec![facts], 1));
        // A2 fires too: the observed NULL, combined with a rendering that
        // is not optional, is a real crash-on-decode risk -- not just an
        // analyzer/codegen disagreement.
        assert_eq!(
            v.failures,
            vec![
                Failure::FidelityMismatch {
                    column: "total".to_string(),
                    analyzed_nullable: true,
                    generated_nullable: false,
                },
                Failure::UnsoundNullability {
                    column: "total".to_string(),
                    row: 0,
                },
            ]
        );
    }

    fn facts_from(columns: Vec<ColumnFacts>, row_count: usize) -> FixtureEngineFacts {
        FixtureEngineFacts::new("live_test", Engine::Postgresql, columns, row_count).unwrap()
    }

    #[test]
    fn new_rejects_a_column_whose_observed_nulls_length_disagrees_with_row_count() {
        let result = FixtureEngineFacts::new(
            "live_test",
            Engine::Postgresql,
            vec![column("total", true, true, vec![true])],
            2,
        );
        assert_eq!(
            result.unwrap_err(),
            RowCountMismatch {
                column: "total".to_string(),
                expected: 2,
                actual: 1,
            }
        );
    }

    // -- A2 soundness ----------------------------------------------------

    #[test]
    fn a2_passes_when_non_nullable_column_is_never_observed_null() {
        let v = evaluate(&facts(vec![column("id", false, false, vec![false, false])], 2));
        assert!(v.is_pass(), "{:?}", v.failures);
    }

    #[test]
    fn a2_fires_when_non_nullable_column_is_observed_null() {
        let v = evaluate(&facts(vec![column("id", false, false, vec![false, true, false])], 3));
        assert_eq!(
            v.failures,
            vec![Failure::UnsoundNullability {
                column: "id".to_string(),
                row: 1
            }]
        );
    }

    #[test]
    fn a2_fires_once_per_offending_row() {
        let v = evaluate(&facts(vec![column("id", false, false, vec![true, false, true])], 3));
        assert_eq!(
            v.failures,
            vec![
                Failure::UnsoundNullability {
                    column: "id".to_string(),
                    row: 0
                },
                Failure::UnsoundNullability {
                    column: "id".to_string(),
                    row: 2
                },
            ]
        );
    }

    #[test]
    fn a2_keys_on_generated_nullable_not_analyzed_nullable() {
        // analyzed says nullable, generated renders non-optional (a fidelity
        // bug caught separately by A1) -- the crash risk is real (generated
        // code decodes non-optionally), so A2 must still fire.
        let v = evaluate(&facts(vec![column("total", true, false, vec![true])], 1));
        assert!(
            v.failures.contains(&Failure::UnsoundNullability {
                column: "total".to_string(),
                row: 0
            }),
            "{:?}",
            v.failures
        );
    }

    #[test]
    fn a2_does_not_fire_when_generated_is_nullable_even_if_analyzed_is_not() {
        // analyzed says non-nullable, generated renders optional (codegen
        // erred safe) -- no crash risk, so A2 must stay quiet even though
        // the engine returned NULL.
        let v = evaluate(&facts(vec![column("total", false, true, vec![true])], 1));
        assert!(
            !v.failures.contains(&Failure::UnsoundNullability {
                column: "total".to_string(),
                row: 0
            }),
            "{:?}",
            v.failures
        );
    }

    // -- A3 anti-vacuity ---------------------------------------------------

    #[test]
    fn a3_passes_when_nullable_column_is_demonstrated_null() {
        let v = evaluate(&facts(vec![column("total", true, true, vec![false, true])], 2));
        assert!(v.is_pass(), "{:?}", v.failures);
    }

    #[test]
    fn a3_fires_when_nullable_column_is_never_demonstrated_null() {
        let v = evaluate(&facts(vec![column("total", true, true, vec![false, false])], 2));
        assert_eq!(
            v.failures,
            vec![Failure::VacuousNullable {
                column: "total".to_string()
            }]
        );
    }

    #[test]
    fn a3_does_not_fire_for_non_nullable_columns() {
        let v = evaluate(&facts(vec![column("id", false, false, vec![false, false])], 2));
        assert!(v.is_pass(), "{:?}", v.failures);
    }

    // -- A4 join-group coherence --------------------------------------

    fn joined_column(name: &str, group: &str, nullable_before_join: bool, observed_nulls: Vec<bool>) -> ColumnFacts {
        ColumnFacts {
            column: name.to_string(),
            analyzed_nullable: true,
            generated_nullable: true,
            join_group: Some(group.to_string()),
            nullable_before_join,
            observed_nulls,
        }
    }

    #[test]
    fn a4_passes_when_group_is_null_together() {
        let v = evaluate(&facts(
            vec![
                joined_column("total", "o", false, vec![false, true]),
                joined_column("notes", "o", false, vec![false, true]),
            ],
            2,
        ));
        assert!(v.is_pass(), "{:?}", v.failures);
    }

    #[test]
    fn a4_fires_when_group_disagrees_on_a_row() {
        // Row 1 is the incoherent one (total null, notes not); row 2 is
        // coherent (both null) and gives `notes` its own anti-vacuity
        // witness so only A4 is under test.
        let v = evaluate(&facts(
            vec![
                joined_column("total", "o", false, vec![false, true, true]),
                joined_column("notes", "o", false, vec![false, false, true]),
            ],
            3,
        ));
        assert_eq!(
            v.failures,
            vec![Failure::JoinGroupIncoherent {
                group: "o".to_string(),
                row: 1,
                columns: vec!["total".to_string(), "notes".to_string()],
            }]
        );
    }

    #[test]
    fn a4_ignores_columns_that_were_already_nullable_before_the_join() {
        // ~keep `nullable_before_join == true` columns aren't reliable
        // discriminants, so a solo disagreement here must not fire A4.
        let v = evaluate(&facts(
            vec![
                joined_column("total", "o", false, vec![false, true]),
                joined_column("notes", "o", true, vec![true, true]),
            ],
            2,
        ));
        assert!(v.is_pass(), "{:?}", v.failures);
    }

    #[test]
    fn a4_ignores_solo_group_members() {
        let v = evaluate(&facts(vec![joined_column("total", "o", false, vec![false, true])], 2));
        assert!(v.is_pass(), "{:?}", v.failures);
    }

    #[test]
    fn combined_failures_are_all_reported_in_one_pass() {
        let v = evaluate(&facts(
            vec![
                column("id", false, false, vec![true]),   // A2
                column("total", true, true, vec![false]), // A3
                // A1 (analyzed/generated disagree) *and* A2 (generated
                // renders non-optional, and it was observed null) -- A2 is
                // keyed on the generated side, so a fidelity mismatch that
                // under-renders nullability is also a soundness failure.
                column("name", true, false, vec![true]),
            ],
            1,
        ));
        assert_eq!(v.failures.len(), 4, "{:?}", v.failures);
    }
}
