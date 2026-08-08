//! Shared file-header machinery for the four Go backends
//! (`go-database-sql`, `go-godror`, `go-gosnowflake`, `go-pgx`).
//!
//! Every generated Go file needs `"context"` (every function takes
//! `ctx context.Context`) plus whatever driver package the backend binds
//! against, but `"time"` and `"github.com/shopspring/decimal"` are only
//! needed when a generated fragment actually produces a `time.Time` or
//! `decimal.Decimal` value -- which depends on whether the schema has any
//! temporal/decimal columns (see the `date`/`time`/`datetime*` and
//! `decimal` entries in each backend's `manifests/go-*.toml`). Emitting
//! those two imports unconditionally makes `go build` fail with "imported
//! and not used" on a schema with neither (#100).

use std::fmt::Write as _;

use crate::GeneratedCode;

/// `github.com/shopspring/decimal`'s import path, the only third-party
/// package any Go backend here pulls in purely to represent a scalar type
/// (`go-pgx`'s manifest maps `decimal` to `decimal.Decimal`; every other Go
/// backend maps `decimal` to `float64` and never needs this import).
const DECIMAL_IMPORT: &str = "github.com/shopspring/decimal";

/// Build a Go file header: `preamble` (doc comment lines, if any, plus the
/// `package` line -- kept verbatim per backend since they disagree on
/// whether/how to emit the "Code generated"/"goimports" comments) followed
/// by an `import (...)` block.
///
/// `stdlib_imports` and `third_party_imports` are import paths every
/// function this backend emits unconditionally needs (e.g. `"context"`,
/// `"database/sql"`, `"github.com/jackc/pgx/v5/pgxpool"`). `"time"` is
/// appended to the stdlib group and [`DECIMAL_IMPORT`] to the third-party
/// group only when `uses_time` / `uses_decimal` say a generated fragment for
/// this file actually references them -- see [`generated_code_uses_time`] /
/// [`generated_code_uses_decimal`].
///
/// A blank-line-separated third-party group is only emitted when
/// `third_party_imports` is non-empty or `uses_decimal` is set; backends
/// with no third-party imports (`go-database-sql`, `go-godror`,
/// `go-gosnowflake`) therefore keep their single-group `import (...)` block.
pub(crate) fn go_file_header(
    preamble: &str,
    stdlib_imports: &[&str],
    third_party_imports: &[&str],
    uses_time: bool,
    uses_decimal: bool,
) -> String {
    let mut header = String::from(preamble);
    header.push_str("\n\nimport (\n");

    for import in stdlib_imports {
        let _ = writeln!(header, "\t\"{import}\"");
    }
    if uses_time {
        header.push_str("\t\"time\"\n");
    }

    if !third_party_imports.is_empty() || uses_decimal {
        header.push('\n');
        for import in third_party_imports {
            let _ = writeln!(header, "\t\"{import}\"");
        }
        if uses_decimal {
            let _ = writeln!(header, "\t\"{DECIMAL_IMPORT}\"");
        }
    }

    header.push_str(")\n");
    header
}

/// Whether any fragment of already-generated code for this file references
/// `needle` (a package selector prefix such as `"time."` or `"decimal."`).
///
/// Checks the same fragments the file assembler concatenates
/// (`enum_def`/`model_struct`/`row_struct`/`query_fn`) plus
/// `nested_struct_defs` -- the JSON/JSONB-nested-aggregate struct
/// definitions `go-pgx` opts into (see
/// [`crate::backends::engine_supports_nested_aggregates`]) -- since a nested struct's
/// fields go through the same scalar type mapping as a row struct's and can
/// just as easily be the *only* place `time.Time`/`decimal.Decimal` appears
/// in a file.
fn generated_code_contains(code: &GeneratedCode, needle: &str) -> bool {
    [
        code.enum_def.as_deref(),
        code.model_struct.as_deref(),
        code.row_struct.as_deref(),
        code.query_fn.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|fragment| fragment.contains(needle))
        || code.nested_struct_defs.iter().any(|def| def.code.contains(needle))
}

/// Whether any generated fragment in `generated` references `time.Time`.
pub(crate) fn generated_code_uses_time(generated: &[GeneratedCode]) -> bool {
    generated.iter().any(|code| generated_code_contains(code, "time."))
}

/// Whether any generated fragment in `generated` references `decimal.Decimal`.
pub(crate) fn generated_code_uses_decimal(generated: &[GeneratedCode]) -> bool {
    generated.iter().any(|code| generated_code_contains(code, "decimal."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_with_row_struct(row_struct: &str) -> GeneratedCode {
        GeneratedCode::build(|c| {
            c.row_struct = Some(row_struct.to_string());
        })
    }

    #[test]
    fn test_go_file_header_single_group_omits_time_when_unused() {
        let header = go_file_header("package queries", &["context", "database/sql"], &[], false, false);
        assert_eq!(
            header,
            "package queries\n\nimport (\n\t\"context\"\n\t\"database/sql\"\n)\n"
        );
    }

    #[test]
    fn test_go_file_header_single_group_includes_time_when_used() {
        let header = go_file_header("package queries", &["context", "database/sql"], &[], true, false);
        assert_eq!(
            header,
            "package queries\n\nimport (\n\t\"context\"\n\t\"database/sql\"\n\t\"time\"\n)\n"
        );
    }

    #[test]
    fn test_go_file_header_two_group_matches_pgx_fixture_when_both_used() {
        let preamble =
            "// Code generated by scythe. DO NOT EDIT.\n// Run `goimports -w .` to fix imports.\npackage queries";
        let header = go_file_header(preamble, &["context"], &["github.com/jackc/pgx/v5/pgxpool"], true, true);
        assert_eq!(
            header,
            "// Code generated by scythe. DO NOT EDIT.\n// Run `goimports -w .` to fix imports.\npackage queries\n\n\
             import (\n\t\"context\"\n\t\"time\"\n\n\t\"github.com/jackc/pgx/v5/pgxpool\"\n\t\"github.com/shopspring/\
             decimal\"\n)\n"
        );
    }

    #[test]
    fn test_go_file_header_two_group_keeps_third_party_group_without_decimal() {
        let header = go_file_header(
            "package queries",
            &["context"],
            &["github.com/jackc/pgx/v5/pgxpool"],
            false,
            false,
        );
        assert_eq!(
            header,
            "package queries\n\nimport (\n\t\"context\"\n\n\t\"github.com/jackc/pgx/v5/pgxpool\"\n)\n"
        );
    }

    #[test]
    fn test_generated_code_uses_time_false_for_int_and_string_only() {
        let generated = [code_with_row_struct("type Row struct {\n\tId int32\n\tName string\n}")];
        assert!(!generated_code_uses_time(&generated));
        assert!(!generated_code_uses_decimal(&generated));
    }

    #[test]
    fn test_generated_code_uses_time_true_when_time_time_present() {
        let generated = [code_with_row_struct("type Row struct {\n\tCreatedAt time.Time\n}")];
        assert!(generated_code_uses_time(&generated));
        assert!(!generated_code_uses_decimal(&generated));
    }

    #[test]
    fn test_generated_code_uses_decimal_true_when_decimal_decimal_present() {
        let generated = [code_with_row_struct("type Row struct {\n\tTotal decimal.Decimal\n}")];
        assert!(!generated_code_uses_time(&generated));
        assert!(generated_code_uses_decimal(&generated));
    }

    #[test]
    fn test_generated_code_uses_time_checks_nested_struct_defs() {
        let generated = [GeneratedCode::build(|c| {
            c.nested_struct_defs.push(crate::NestedStructDef {
                name: "nested".to_string(),
                code: "type Nested struct {\n\tSeenAt time.Time\n}".to_string(),
            });
        })];
        assert!(generated_code_uses_time(&generated));
    }
}
