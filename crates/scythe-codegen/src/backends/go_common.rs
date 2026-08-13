//! Shared file-header machinery for the four Go backends
//! (`go-database-sql`, `go-godror`, `go-gosnowflake`, `go-pgx`).
//!
//! Every generated Go file needs `"context"` (every function takes
//! `ctx context.Context`) plus whatever driver package the backend binds
//! against. Everything else -- `"time"`, `"github.com/shopspring/decimal"`,
//! and (for `go-pgx`) `"github.com/google/uuid"`, `"encoding/json"`,
//! `"net/netip"` -- is needed only when a generated fragment actually
//! produces the type that import backs, which depends on whether the schema
//! has any column of that type (see each backend's `manifests/go-*.toml`).
//! Emitting an import unconditionally makes `go build` fail with "imported
//! and not used" on a schema that never needs it (#100); never emitting one
//! a generated fragment does need makes `go build` fail with "undefined:
//! uuid" instead (#198).
//!
//! Which imports fall into the second category, and what package path backs
//! each one, comes entirely from the manifest's own `[imports.rules]` --
//! never from a table hardcoded here. `go-pgx.toml` maps `uuid`, `json`, and
//! `inet` to types from three different packages; every other Go manifest
//! maps them to a Go primitive or to `time.Time`/`string` and declares no
//! rule for them at all, and this module never needs to know that split --
//! it just asks the manifest.

use std::fmt::Write as _;

use scythe_backend::manifest::BackendManifest;

use crate::GeneratedCode;

/// Build a Go file header: `preamble` (doc comment lines, if any, plus the
/// `package` line -- kept verbatim per backend since they disagree on
/// whether/how to emit the "Code generated"/"goimports" comments) followed
/// by an `import (...)` block.
///
/// `stdlib_imports` and `third_party_imports` are import paths every
/// function this backend emits unconditionally needs (e.g. `"context"`,
/// `"database/sql"`, `"github.com/jackc/pgx/v5/pgxpool"`). Anything
/// `manifest`'s `[imports.rules]` declares is added on top of those, but
/// only for a rule whose key (a package-selector prefix such as `"time."`
/// or `"uuid."`) actually appears somewhere in `generated` -- see
/// [`resolve_manifest_imports`].
///
/// A blank-line-separated third-party group is only emitted when
/// `third_party_imports` is non-empty or a manifest-driven rule resolved to
/// a third-party import; backends with no third-party imports and no such
/// rule (`go-database-sql`, `go-godror`, `go-gosnowflake` outside their
/// `"time."` rule) therefore keep their single-group `import (...)` block.
pub(crate) fn go_file_header(
    preamble: &str,
    stdlib_imports: &[&str],
    third_party_imports: &[&str],
    manifest: &BackendManifest,
    generated: &[GeneratedCode],
) -> String {
    let (extra_stdlib, extra_third_party) = resolve_manifest_imports(manifest, generated);

    let mut header = String::from(preamble);
    header.push_str("\n\nimport (\n");

    for import in stdlib_imports {
        let _ = writeln!(header, "\t\"{import}\"");
    }
    for import in &extra_stdlib {
        let _ = writeln!(header, "\t{import}");
    }

    if !third_party_imports.is_empty() || !extra_third_party.is_empty() {
        header.push('\n');
        for import in third_party_imports {
            let _ = writeln!(header, "\t\"{import}\"");
        }
        for import in &extra_third_party {
            let _ = writeln!(header, "\t{import}");
        }
    }

    header.push_str(")\n");
    header
}

/// The extra stdlib and third-party imports a generated file needs beyond
/// `stdlib_imports`/`third_party_imports`, derived from `manifest.imports`
/// rather than a hardcoded table.
///
/// Before this existed, `go_file_header` hardcoded `"time"` and
/// `github.com/shopspring/decimal` as the only two conditional imports and
/// consulted nothing else, even though `go-pgx.toml` also declares
/// `[imports.rules]` entries for `uuid.`, `json.`, and `netip.`. A `uuid`,
/// `json`, or `inet` column emitted `uuid.UUID` / `json.RawMessage` /
/// `netip.Addr` with no import for any of them, and the generated file did
/// not compile (#198).
///
/// A manifest with an empty or absent `[imports.rules]` -- every Go manifest
/// except `go-pgx`/`go-pgx.redshift`, whose scalars other than `decimal`
/// resolve to Go primitives or `time.Time` -- simply contributes nothing
/// beyond what `stdlib_imports`/`third_party_imports` already name; the
/// caller's unconditional imports are unaffected either way.
///
/// Returned as `(stdlib, third_party)`, each sorted and deduplicated: rule
/// iteration order comes from an `AHashMap` and is not itself stable across
/// runs, and a sorted header is what makes the generated file's import block
/// reproducible.
fn resolve_manifest_imports(manifest: &BackendManifest, generated: &[GeneratedCode]) -> (Vec<String>, Vec<String>) {
    let mut stdlib = Vec::new();
    let mut third_party = Vec::new();

    if let Some(imports) = &manifest.imports {
        for (prefix, import) in &imports.rules {
            if !generated_code_uses_prefix(generated, prefix) {
                continue;
            }
            if is_stdlib_import(import) {
                stdlib.push(import.clone());
            } else {
                third_party.push(import.clone());
            }
        }
    }

    stdlib.sort();
    stdlib.dedup();
    third_party.sort();
    third_party.dedup();
    (stdlib, third_party)
}

/// Whether `import` (a manifest rule's value, e.g. `"time"` or
/// `"github.com/google/uuid"`, already carrying the quotes Go's import block
/// needs around a path) belongs in Go's stdlib import group.
///
/// Uses the same heuristic `gofmt`/`goimports` group imports by: a
/// third-party path's first segment names a registrable domain and so
/// contains a `.` (`github.com/...`); no stdlib path's first segment does,
/// including the ones that also contain a `/` (`"encoding/json"`,
/// `"net/netip"`).
fn is_stdlib_import(import: &str) -> bool {
    let path = import.trim_matches('"');
    let first_segment = path.split('/').next().unwrap_or(path);
    !first_segment.contains('.')
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
/// just as easily be the *only* place a package selector like `time.` or
/// `uuid.` appears in a file.
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

/// Whether any generated fragment in `generated` references `needle`.
fn generated_code_uses_prefix(generated: &[GeneratedCode], needle: &str) -> bool {
    generated.iter().any(|code| generated_code_contains(code, needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal but structurally valid Go manifest, with `[imports.rules]`
    /// populated from `rules`. Built from a real TOML string (rather than by
    /// hand-constructing a `BackendManifest`) so the test exercises the same
    /// parse path -- and the same rule-value quoting convention (values
    /// already carry the quotes Go's import block needs) -- that
    /// `manifests/go-*.toml` does.
    fn manifest_with_rules(rules: &[(&str, &str)]) -> BackendManifest {
        let mut toml = String::from(
            "[backend]\nname = \"test\"\nlanguage = \"go\"\nfile_extension = \"go\"\nengine = \"test\"\n\n\
             [types.scalars]\n[types.containers]\n\n\
             [naming]\nstruct_case = \"PascalCase\"\nfn_case = \"PascalCase\"\n\
             enum_variant_case = \"PascalCase\"\nrow_suffix = \"Row\"\n\n[imports.rules]\n",
        );
        for (key, value) in rules {
            let _ = writeln!(toml, "{key:?} = {value:?}");
        }
        toml::from_str(&toml).expect("test manifest must parse")
    }

    /// A manifest with no `[imports]` section at all (`manifest.imports ==
    /// None`), distinct from [`manifest_with_rules`] called with an empty
    /// rule list (`manifest.imports == Some(rules: {})`) -- both must
    /// resolve to "no extra imports".
    fn manifest_without_imports() -> BackendManifest {
        toml::from_str(
            "[backend]\nname = \"test\"\nlanguage = \"go\"\nfile_extension = \"go\"\nengine = \"test\"\n\n\
             [types.scalars]\n[types.containers]\n\n\
             [naming]\nstruct_case = \"PascalCase\"\nfn_case = \"PascalCase\"\n\
             enum_variant_case = \"PascalCase\"\nrow_suffix = \"Row\"\n",
        )
        .expect("test manifest must parse")
    }

    fn code_with_row_struct(row_struct: &str) -> GeneratedCode {
        GeneratedCode::build(|c| {
            c.row_struct = Some(row_struct.to_string());
        })
    }

    #[test]
    fn test_go_file_header_single_group_omits_extras_when_manifest_has_no_rules() {
        let manifest = manifest_without_imports();
        let header = go_file_header("package queries", &["context", "database/sql"], &[], &manifest, &[]);
        assert_eq!(
            header,
            "package queries\n\nimport (\n\t\"context\"\n\t\"database/sql\"\n)\n"
        );
    }

    #[test]
    fn test_go_file_header_single_group_includes_time_when_generated_code_uses_it() {
        let manifest = manifest_with_rules(&[("time.", "\"time\"")]);
        let generated = [code_with_row_struct("type Row struct {\n\tCreatedAt time.Time\n}")];
        let header = go_file_header(
            "package queries",
            &["context", "database/sql"],
            &[],
            &manifest,
            &generated,
        );
        assert_eq!(
            header,
            "package queries\n\nimport (\n\t\"context\"\n\t\"database/sql\"\n\t\"time\"\n)\n"
        );
    }

    #[test]
    fn test_go_file_header_two_group_matches_pgx_fixture_when_time_and_decimal_used() {
        let preamble =
            "// Code generated by scythe. DO NOT EDIT.\n// Run `goimports -w .` to fix imports.\npackage queries";
        let manifest = manifest_with_rules(&[("time.", "\"time\""), ("decimal.", "\"github.com/shopspring/decimal\"")]);
        let generated = [code_with_row_struct(
            "type Row struct {\n\tCreatedAt time.Time\n\tTotal decimal.Decimal\n}",
        )];
        let header = go_file_header(
            preamble,
            &["context"],
            &["github.com/jackc/pgx/v5/pgxpool"],
            &manifest,
            &generated,
        );
        assert_eq!(
            header,
            "// Code generated by scythe. DO NOT EDIT.\n// Run `goimports -w .` to fix imports.\npackage queries\n\n\
             import (\n\t\"context\"\n\t\"time\"\n\n\t\"github.com/jackc/pgx/v5/pgxpool\"\n\t\"github.com/shopspring/\
             decimal\"\n)\n"
        );
    }

    #[test]
    fn test_go_file_header_two_group_keeps_third_party_group_without_manifest_extras() {
        let manifest = manifest_without_imports();
        let header = go_file_header(
            "package queries",
            &["context"],
            &["github.com/jackc/pgx/v5/pgxpool"],
            &manifest,
            &[],
        );
        assert_eq!(
            header,
            "package queries\n\nimport (\n\t\"context\"\n\n\t\"github.com/jackc/pgx/v5/pgxpool\"\n)\n"
        );
    }

    /// The defect this whole file exists to fix (#198): `go-pgx.toml` names
    /// three more rules than the old hardcoded table knew about. All three
    /// must show up, `uuid.` and `json.`/`netip.` split correctly across the
    /// stdlib/third-party groups even though all three are declared in the
    /// same `[imports.rules]` table.
    #[test]
    fn test_go_file_header_adds_stdlib_and_third_party_imports_for_uuid_json_and_netip() {
        let manifest = manifest_with_rules(&[
            ("uuid.", "\"github.com/google/uuid\""),
            ("json.", "\"encoding/json\""),
            ("netip.", "\"net/netip\""),
        ]);
        let generated = [code_with_row_struct(
            "type Row struct {\n\tId uuid.UUID\n\tPayload json.RawMessage\n\tIp netip.Addr\n}",
        )];
        let header = go_file_header(
            "package queries",
            &["context"],
            &["github.com/jackc/pgx/v5/pgxpool"],
            &manifest,
            &generated,
        );
        assert_eq!(
            header,
            "package queries\n\nimport (\n\t\"context\"\n\t\"encoding/json\"\n\t\"net/netip\"\n\n\
             \t\"github.com/jackc/pgx/v5/pgxpool\"\n\t\"github.com/google/uuid\"\n)\n"
        );
    }

    #[test]
    fn test_go_file_header_omits_rule_whose_prefix_never_appears_in_generated_code() {
        let manifest = manifest_with_rules(&[("decimal.", "\"github.com/shopspring/decimal\"")]);
        let generated = [code_with_row_struct("type Row struct {\n\tId int32\n\tName string\n}")];
        let header = go_file_header(
            "package queries",
            &["context", "database/sql"],
            &[],
            &manifest,
            &generated,
        );
        assert_eq!(
            header,
            "package queries\n\nimport (\n\t\"context\"\n\t\"database/sql\"\n)\n"
        );
    }

    #[test]
    fn test_generated_code_uses_prefix_checks_nested_struct_defs() {
        let generated = [GeneratedCode::build(|c| {
            c.nested_struct_defs.push(crate::NestedStructDef {
                name: "nested".to_string(),
                code: "type Nested struct {\n\tSeenAt time.Time\n}".to_string(),
            });
        })];
        assert!(generated_code_uses_prefix(&generated, "time."));
    }

    #[test]
    fn test_is_stdlib_import_distinguishes_by_first_path_segment() {
        assert!(is_stdlib_import("\"time\""));
        assert!(is_stdlib_import("\"encoding/json\""));
        assert!(is_stdlib_import("\"net/netip\""));
        assert!(!is_stdlib_import("\"github.com/google/uuid\""));
        assert!(!is_stdlib_import("\"github.com/shopspring/decimal\""));
    }
}
