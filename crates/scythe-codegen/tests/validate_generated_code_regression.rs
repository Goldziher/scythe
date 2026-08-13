//! Regression coverage for `validate_generated_code`, the CLI-facing
//! collapse of `validate_with_tools` into three reportable outcomes.
//!
//! `validate_with_tools`/`ToolValidation` had no production caller: every
//! call site outside `validation.rs` was a test, so `scythe generate` never
//! checked its own output with a real compiler even though the machinery
//! existed. `validate_generated_code` is the API a CLI `--verify` flag is
//! meant to call. These tests exercise the three outcomes it must be able to
//! report -- no validator for the backend's language, a validator that ran
//! clean, and a validator that caught deliberately broken input -- without
//! assuming any particular tool is installed in the environment running
//! `cargo test`, since a missing tool is tolerated (not a failure) under the
//! non-strict policy this function is built on.

use scythe_codegen::validation::{ValidationOutcome, validate_generated_code};

/// `csharp-npgsql` is one of the languages `validate_with_tools` has no
/// checker for at all (see `NO_TOOL_VALIDATOR` in `tests/tool_validation.rs`)
/// -- `using Npgsql;` with no compiled `Npgsql.dll` on the reference path is
/// a hard `CS0246`, and six distinct NuGet-only driver APIs is too much
/// surface to hand-stub credibly. `Unsupported` must never read as a pass:
/// that collapse is exactly the bug #98 (`ToolValidation`) exists to
/// prevent, and this is the case a `--verify` flag must not silently skip
/// past when it reports results.
#[test]
fn no_validator_for_the_backends_language_reports_unsupported() {
    let outcome = validate_generated_code("public class Foo {}", "csharp-npgsql");

    assert_eq!(outcome, ValidationOutcome::Unsupported);
    assert!(
        !outcome.is_failure(),
        "a language with no validator must not block a build -- see ValidationOutcome::is_failure"
    );
}

/// `python-psycopg3` is checked by `poly`'s bundled `ruff` (see
/// `validate_python_tools`). Valid Python given to a validator that exists
/// must never come back `Unsupported`, and must never come back `Failed`
/// unless a checker actually ran and found something -- which clean input
/// gives it no grounds to do.
///
/// This does not require `poly` to be installed: if it is absent, nothing
/// ran, `tools_run` is empty, and the non-strict policy still reports
/// `Passed` (a missing tool is tolerated, not a failure) -- but a `Failed`
/// result here would always be a genuine bug, so that branch is asserted
/// unconditionally.
#[test]
fn a_backend_with_a_validator_reports_passed_for_clean_input() {
    let clean_code = "from dataclasses import dataclass\n\n\n@dataclass\nclass Row:\n    id: int\n";

    let outcome = validate_generated_code(clean_code, "python-psycopg3");

    match outcome {
        ValidationOutcome::Passed { tools_run, .. } => {
            if !tools_run.is_empty() {
                assert!(
                    tools_run.contains(&"poly"),
                    "python-psycopg3 must be checked by poly, got: {tools_run:?}"
                );
            }
        }
        other => panic!("expected Passed for clean python-psycopg3 code, got {other:?}"),
    }
}

/// The same validator, given input that is not valid Python at all. If a
/// checker actually ran (`tools_run` non-empty), it must have caught the
/// syntax error and the outcome must be `Failed` with a non-empty error
/// list -- a validator that runs and finds nothing wrong with `def broken(:`
/// would be the exact false-green signal #98 exists to prevent.
///
/// If nothing was installed to check it (`tools_run` empty on `Passed`),
/// that is the honest, non-strict-mode answer for a laptop without `poly` --
/// this test does not require a specific toolchain -- but it must never
/// silently report `Unsupported`, since a validator does exist for this
/// backend.
#[test]
fn a_backend_with_a_validator_reports_failed_for_broken_input() {
    let broken_code = "def broken(:\n    this is not python\n";

    let outcome = validate_generated_code(broken_code, "python-psycopg3");

    match outcome {
        ValidationOutcome::Failed { tools_run, errors, .. } => {
            assert!(
                !tools_run.is_empty(),
                "a Failed outcome must name at least one checker that actually ran"
            );
            assert!(!errors.is_empty(), "a Failed outcome must carry at least one finding");
        }
        ValidationOutcome::Passed { tools_run, .. } => {
            assert!(
                tools_run.is_empty(),
                "broken Python passed even though a checker ran against it: {tools_run:?}"
            );
        }
        ValidationOutcome::Unsupported => {
            panic!("python-psycopg3 has a real validator and must not report Unsupported")
        }
    }
}
