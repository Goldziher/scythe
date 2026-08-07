//! Resolves the *declared* expectation for a (fixture, run, engine): the
//! run's base `rows` unless `engine_expectations` registers a legitimate
//! per-engine semantic override (e.g. Oracle collapsing `''` into `NULL`).
//!
//! This is deliberately separate from [`crate::divergence`]:
//! `engine_expectations` changes what a fixture *asserts* before any
//! comparison happens, while the divergence registry suppresses specific
//! assertion *failures* after the fact. Neither mechanism can be used to
//! explain away an A2 soundness failure -- `engine_expectations` is about
//! what the fixture author expects the data to look like, not about
//! silencing a mismatch between the analyzer and reality.

use crate::fixture::{Engine, LiveBlock, RowExpectation, Run};

/// Resolve the effective per-row expectation for `run` under `engine`: the
/// `engine_expectations` override for this run if one is registered,
/// otherwise the run's base `rows`.
pub fn resolve_run_rows<'a>(live: &'a LiveBlock, run: &'a Run, engine: Engine) -> &'a [RowExpectation] {
    live.engine_expectations
        .get(&engine)
        .and_then(|exp| exp.runs.get(&run.name))
        .map(Vec::as_slice)
        .unwrap_or(&run.rows)
}

#[derive(Debug, thiserror::Error)]
pub enum ExpectationError {
    #[error("engine_expectations references engine {engine} which is not in this fixture's engines list")]
    UnknownEngine { engine: Engine },
    #[error("engine_expectations.{engine}.runs references run {run:?} which does not exist")]
    UnknownRun { engine: Engine, run: String },
    #[error(
        "engine_expectations.{engine}.runs[{run:?}] declares {override_count} row(s) but the base run declares {base} -- an override changes what a row asserts, not how many rows exist"
    )]
    RowCountMismatch {
        engine: Engine,
        run: String,
        base: usize,
        override_count: usize,
    },
}

/// Rejects `engine_expectations` entries that reference an engine this
/// fixture doesn't test, a run name that doesn't exist, or an override
/// whose row count disagrees with the base run's -- an orphaned or
/// mismatched override is a fixture-authoring bug, not something to skip or
/// discover only when a live run's row count happens to disagree with the
/// wrong list (see [`crate::fixture::Run::check_row_count`], which compares
/// against whichever row list this function has already validated is the
/// right length).
pub fn validate(live: &LiveBlock) -> Result<(), ExpectationError> {
    for (&engine, exp) in &live.engine_expectations {
        if !live.engines.contains(&engine) {
            return Err(ExpectationError::UnknownEngine { engine });
        }
        for (run_name, override_rows) in &exp.runs {
            let Some(base_run) = live.runs.iter().find(|r| &r.name == run_name) else {
                return Err(ExpectationError::UnknownRun {
                    engine,
                    run: run_name.clone(),
                });
            };
            if override_rows.len() != base_run.rows.len() {
                return Err(ExpectationError::RowCountMismatch {
                    engine,
                    run: run_name.clone(),
                    base: base_run.rows.len(),
                    override_count: override_rows.len(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{EngineExpectation, RowExpectation, SeedBlock};
    use ahash::AHashMap;

    fn run(name: &str, rows: Vec<RowExpectation>) -> Run {
        Run {
            name: name.to_string(),
            seed: SeedBlock {
                default: vec!["INSERT".to_string()],
                per_engine: Default::default(),
            },
            rows,
        }
    }

    fn base_row() -> RowExpectation {
        RowExpectation {
            non_null: vec!["id".to_string()],
            null: vec![],
        }
    }

    fn override_row() -> RowExpectation {
        RowExpectation {
            non_null: vec![],
            null: vec!["id".to_string()],
        }
    }

    fn live_block(runs: Vec<Run>, engine_expectations: AHashMap<Engine, EngineExpectation>) -> LiveBlock {
        LiveBlock {
            schema_profile: "profile".to_string(),
            engines: vec![Engine::Oracle],
            runs,
            null_together: vec![],
            engine_expectations,
        }
    }

    // -- resolve_run_rows ----------------------------------------------

    #[test]
    fn resolve_run_rows_uses_base_rows_when_no_override_is_registered() {
        let r = run("run1", vec![base_row()]);
        let live = live_block(vec![r.clone()], AHashMap::new());
        let resolved = resolve_run_rows(&live, &r, Engine::Oracle);
        assert_eq!(resolved, &[base_row()]);
    }

    #[test]
    fn resolve_run_rows_prefers_a_registered_engine_override() {
        let r = run("run1", vec![base_row()]);
        let mut runs = AHashMap::new();
        runs.insert("run1".to_string(), vec![override_row()]);
        let exp = EngineExpectation {
            reason: "Oracle collapses '' into NULL".to_string(),
            runs,
        };
        let mut engine_expectations = AHashMap::new();
        engine_expectations.insert(Engine::Oracle, exp);
        let live = live_block(vec![r.clone()], engine_expectations);
        let resolved = resolve_run_rows(&live, &r, Engine::Oracle);
        assert_eq!(resolved, &[override_row()]);
    }

    #[test]
    fn resolve_run_rows_ignores_an_override_registered_for_a_different_run() {
        let r = run("run1", vec![base_row()]);
        let mut runs = AHashMap::new();
        runs.insert("some_other_run".to_string(), vec![override_row()]);
        let exp = EngineExpectation {
            reason: "test".to_string(),
            runs,
        };
        let mut engine_expectations = AHashMap::new();
        engine_expectations.insert(Engine::Oracle, exp);
        let live = live_block(vec![r.clone()], engine_expectations);
        let resolved = resolve_run_rows(&live, &r, Engine::Oracle);
        assert_eq!(resolved, &[base_row()]);
    }

    // -- validate ---------------------------------------------------------

    #[test]
    fn validate_accepts_an_override_for_a_listed_engine_and_existing_run() {
        let r = run("run1", vec![base_row()]);
        let mut runs = AHashMap::new();
        runs.insert("run1".to_string(), vec![override_row()]);
        let exp = EngineExpectation {
            reason: "test".to_string(),
            runs,
        };
        let mut engine_expectations = AHashMap::new();
        engine_expectations.insert(Engine::Oracle, exp);
        let live = live_block(vec![r], engine_expectations);
        assert!(validate(&live).is_ok());
    }

    #[test]
    fn validate_rejects_an_override_for_an_engine_not_in_the_engines_list() {
        let r = run("run1", vec![base_row()]);
        let mut runs = AHashMap::new();
        runs.insert("run1".to_string(), vec![override_row()]);
        let exp = EngineExpectation {
            reason: "test".to_string(),
            runs,
        };
        let mut engine_expectations = AHashMap::new();
        engine_expectations.insert(Engine::Mssql, exp); // live_block only lists Oracle
        let live = live_block(vec![r], engine_expectations);
        assert!(matches!(
            validate(&live),
            Err(ExpectationError::UnknownEngine { engine: Engine::Mssql })
        ));
    }

    #[test]
    fn validate_rejects_an_override_that_references_a_nonexistent_run() {
        let r = run("run1", vec![base_row()]);
        let mut runs = AHashMap::new();
        runs.insert("no_such_run".to_string(), vec![override_row()]);
        let exp = EngineExpectation {
            reason: "test".to_string(),
            runs,
        };
        let mut engine_expectations = AHashMap::new();
        engine_expectations.insert(Engine::Oracle, exp);
        let live = live_block(vec![r], engine_expectations);
        assert!(matches!(validate(&live), Err(ExpectationError::UnknownRun { .. })));
    }

    #[test]
    fn validate_rejects_an_override_whose_row_count_disagrees_with_the_base_run() {
        let r = run("run1", vec![base_row(), base_row()]); // base declares 2 rows
        let mut runs = AHashMap::new();
        runs.insert("run1".to_string(), vec![override_row()]); // override declares 1
        let exp = EngineExpectation {
            reason: "test".to_string(),
            runs,
        };
        let mut engine_expectations = AHashMap::new();
        engine_expectations.insert(Engine::Oracle, exp);
        let live = live_block(vec![r], engine_expectations);
        assert!(matches!(
            validate(&live),
            Err(ExpectationError::RowCountMismatch {
                engine: Engine::Oracle,
                base: 2,
                override_count: 1,
                ..
            })
        ));
    }
}
