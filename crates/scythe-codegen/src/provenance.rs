//! The provenance header line every generated file carries, and the exact
//! byte ordering it sits in.
//!
//! This lives in `scythe-codegen` rather than in the `scythe` CLI that
//! actually writes the files, for one reason: `tests/tool_validation.rs` in
//! this crate hands assembled files to the real language tools (`php -l`,
//! `ruby -c`, `gofmt`, ...), and it can only substantiate the claim that the
//! header sits in a position each language accepts if it assembles the same
//! bytes the CLI writes. A second copy of the comment-prefix table, or of
//! the preamble/header/body ordering, kept privately in the test harness,
//! would drift from the real one silently — and it would drift precisely in
//! the case the harness exists to catch.

use std::borrow::Cow;

use crate::backend_trait::CodegenBackend;

/// Line-comment token to embed the provenance header behind, derived from
/// `manifest().backend.language`.
///
/// `language` is a `String`, not a Rust enum, but exactly 10 distinct values
/// are used across every one of the 106 shipped manifests, so this one match
/// table is the single place backends' comment syntax is derived from —
/// no per-backend declaration, and no manifest schema change. Unrecognized
/// values fall back to `//`: every backend shipped today matches one of the
/// listed languages, and `//` is a safe default for any future one that
/// doesn't.
pub fn comment_prefix(language: &str) -> &'static str {
    match language {
        "python" | "ruby" | "elixir" => "#",
        _ => "//",
    }
}

/// Trailing text appended to the header line for languages whose *default*
/// linter configuration rejects it, derived from
/// `manifest().backend.language` the same way [`comment_prefix`] is.
///
/// Python only, for one concrete reason: ruff's default `line-length` is 88,
/// while the header runs to roughly 99 characters (`# scythe:provenance
/// v=0.13.0 backend=python-psycopg3 engine=postgresql schema=sch1:` plus 16
/// hex digits). Without a suppression, *every* file scythe generates for a
/// Python target reports `E501 Line too long` on line 1 under `ruff check
/// --select E` — a lint error the consumer did not write and cannot fix
/// without either hand-editing generated code or loosening their own
/// line-length. `# noqa: E501` is the same mechanism, and the same
/// two-spaces-before-`#` spelling, that scythe's Python backends already use
/// for the `# noqa: F401` on their conditionally-emitted imports.
///
/// The suffix is invisible to the verifier: `parse_provenance_header`
/// tokenizes the text after the sentinel on whitespace and skips any token
/// without an `=`, so `#`, `noqa:`, and `E501` are all ignored and the four
/// `key=value` fields still parse. Pinned by
/// `assemble_output_python_header_carries_noqa_and_still_round_trips` in
/// `scythe-cli`, where the parser lives.
///
/// Shortening the header instead was not an option: the width is driven by
/// the backend name, the engine alias, and a 16-hex-digit fingerprint, none
/// of which can be trimmed without losing information the verifier compares.
///
/// Known trade-off: a project that both raises `line-length` past ~99 *and*
/// opts into `RUF100` (unused-noqa) will see the suppression reported as
/// unnecessary. That is strictly the lesser problem — it takes an opt-in
/// rule plus a non-default line length to hit, whereas E501 fires on ruff's
/// defaults — and it is the same trade-off the existing `# noqa: F401`
/// lines already make.
fn header_suffix(language: &str) -> &'static str {
    match language {
        "python" => "  # noqa: E501",
        _ => "",
    }
}

/// Strip `\n` and `\r` from a provenance field value before it is embedded
/// in the header line.
///
/// Only [`header_line`]'s `engine` argument needs this: `version` is the
/// caller's own package version, `backend.name()` is a hardcoded per-backend
/// literal, and `schema` is always `sch1:` plus 16 lowercase hex characters
/// — none of those three can contain a line terminator. `engine` is the
/// `[[sql]]` `engine = "..."` value, deserialized verbatim from the user's
/// `scythe.toml` with no validation upstream (`normalize_engine` only
/// consults it to pick a dialect; the raw string is what a caller passes
/// through to here). A value containing `\n` or a lone `\r` would terminate
/// the comment early — everything after it would land on its own physical
/// line with no comment prefix, becoming live, uncommented content in the
/// generated file. That breaks the exact guarantee this module is built on:
/// the header always reads as an ordinary comment, never as code.
/// Sanitizing at the point of embedding (rather than at config parse time)
/// means the guarantee holds regardless of how `engine` arrives — this call
/// site today, or any future one — not just for callers that happen to
/// validate it first.
///
/// The verifier (`scythe check`) must sanitize its configured engine the
/// same way before comparing it against the parsed header's `engine` field:
/// the header always holds the sanitized value, so comparing it against a
/// raw, unsanitized value would permanently false-flag SC-PRV04 for any
/// config whose engine string needed sanitizing.
pub fn sanitize_field(value: &str) -> Cow<'_, str> {
    if value.contains(['\n', '\r']) {
        Cow::Owned(value.replace(['\n', '\r'], ""))
    } else {
        Cow::Borrowed(value)
    }
}

/// Build the provenance header line that [`assemble_file`] prepends to every
/// generated file, right after [`CodegenBackend::file_preamble`] and before
/// [`CodegenBackend::file_header`]: the sentinel `scythe check` searches for,
/// commented out using the target language's own line-comment syntax so it
/// reads as an ordinary comment to every downstream compiler, formatter, and
/// human.
///
/// `backend.name()` (not the raw `[[sql.gen]]` `backend = "..."` config
/// value) is what gets embedded — `get_backend` accepts several aliases per
/// backend (`"sqlx"`, `"rust"`, and `"rust-sqlx"` all construct the same
/// backend), and `name()` is the one canonical form every alias agrees on.
/// The verifier's SC-PRV03 check compares against this same `backend.name()`,
/// not the config alias, for exactly this reason.
///
/// `version` is supplied by the caller rather than read from this crate's
/// own `CARGO_PKG_VERSION`. The number that belongs in the header is the
/// version of the `scythe` binary that produced the file, because that is
/// the number `scythe check` compares against; `scythe-codegen` happens to
/// share the workspace version today, but baking its own constant in here
/// would silently embed the wrong one the moment the two diverge.
///
/// `engine` is sanitized via [`sanitize_field`] before embedding — see that
/// function's doc comment for why only `engine` needs it.
///
/// `queries` is the `q1:<16 hex>` fingerprint of the analyzed query set that
/// produced this file (see `AnalyzedQuery::fingerprint_set` in
/// `scythe-core`), the `queries=` counterpart to `schema`'s `sch1:<16 hex>`
/// — added in #94 so that editing a `.sql` query file without touching the
/// schema is no longer invisible to `scythe check`. It is a plain `&str`,
/// not sanitized like `engine`: unlike `engine` (a free-form config value),
/// it is always produced by `fingerprint_set`, which can only ever return
/// the fixed `q1:` tag plus lowercase hex — there is no path by which it
/// could contain a line terminator.
///
/// A per-language [`header_suffix`] may be appended after the last field —
/// today only Python's `# noqa: E501`, without which the line trips ruff's
/// default 88-character limit in every generated file.
pub fn header_line(backend: &dyn CodegenBackend, version: &str, engine: &str, schema: &str, queries: &str) -> String {
    let language = &backend.manifest().backend.language;
    let comment = comment_prefix(language);
    let suffix = header_suffix(language);
    let engine = sanitize_field(engine);
    format!(
        "{comment} scythe:provenance v={version} backend={} engine={engine} schema={schema} queries={queries}{suffix}\n",
        backend.name()
    )
}

/// Join a backend's `preamble` (text that must be the literal first bytes of
/// the file, e.g. PHP's `<?php`), the provenance `header` line, and the
/// generated `body` into the one byte ordering every generated file uses.
///
/// The preamble is unconditionally first — even ahead of the provenance
/// comment — because the constructs [`CodegenBackend::file_preamble`] carries
/// lose their meaning if anything at all precedes them (see that method's
/// doc comment).
///
/// The blank line between the header line and the body is emitted only when
/// `preamble` is non-empty. This is not cosmetic: for the 8 backends with a
/// preamble override, the old (pre-provenance) `file_header()` text already
/// opened with its own blank line (PHP's `"<?php\n\ndeclare(...)"`, Ruby's
/// `"# frozen_string_literal: true\n\n# Auto-generated..."`), so the
/// separator reproduces those exact bytes once the provenance line is
/// accounted for. For the other 44 backends, `file_header()` never had a
/// leading blank line, so an unconditional separator would silently insert
/// one that was never there before — invisible in the assembled output, but
/// a real, provable byte-level regression. Conditioning the separator on
/// `preamble.is_empty()` reproduces the old bytes in both cases:
/// - preamble non-empty (PHP): `"<?php\n"` + header + `"\n"` + `"declare..."`;
///   strip the header line → `"<?php\n\ndeclare..."`, the old bytes exactly.
/// - preamble empty (Go): header + `""` + `"// Code generated..."`;
///   strip the header line → `"// Code generated..."`, the old bytes exactly.
pub fn assemble_file(preamble: &str, header: &str, body: &str) -> String {
    let separator = if preamble.is_empty() { "" } else { "\n" };
    format!("{preamble}{header}{separator}{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_prefix_covers_all_ten_manifest_languages() {
        // The exact 10 values used across every shipped manifest (see
        // `grep -h '^language' crates/scythe-codegen/manifests/*.toml | sort -u`).
        let hash_comment = ["python", "ruby", "elixir"];
        let slash_comment = ["rust", "typescript", "go", "java", "kotlin", "csharp", "php"];

        for language in hash_comment {
            assert_eq!(comment_prefix(language), "#", "language: {language}");
        }
        for language in slash_comment {
            assert_eq!(comment_prefix(language), "//", "language: {language}");
        }
    }

    #[test]
    fn comment_prefix_defaults_to_slash_slash_for_unknown_language() {
        assert_eq!(comment_prefix("cobol"), "//");
    }

    /// Ruff's default `line-length`. The provenance header exceeds it for
    /// every Python backend, which is the whole reason [`header_suffix`]
    /// exists; `validate_python_tools` runs `ruff check --select E,F,I` with
    /// no config file, so this default is what generated files are held to.
    const RUFF_DEFAULT_LINE_LENGTH: usize = 88;

    /// The suppression is Python-specific and must not leak: `# noqa` is a
    /// syntax error in most of the other nine languages, and meaningless
    /// noise in Ruby and Elixir, which share Python's `#` comment token.
    #[test]
    fn header_suffix_is_python_only() {
        assert_eq!(header_suffix("python"), "  # noqa: E501");

        for language in [
            "ruby",
            "elixir",
            "rust",
            "typescript",
            "go",
            "java",
            "kotlin",
            "csharp",
            "php",
            "cobol",
        ] {
            assert_eq!(header_suffix(language), "", "language: {language}");
        }
    }

    /// Every Python backend's header line must either fit ruff's default
    /// line length or carry an `E501` suppression — otherwise `ruff check
    /// --select E` reports `E501 Line too long` on line 1 of every file
    /// scythe generates for that backend.
    ///
    /// Expressed as the real constraint rather than as "ends with the
    /// suffix", so it stays honest if a future backend name is short enough
    /// not to need one. It still fails loudly if the suffix is dropped: the
    /// shortest Python backend name in the tree today still produces a line
    /// well past 88 characters, which the premise assertion below pins.
    #[test]
    fn python_header_lines_never_trip_ruffs_default_line_length() {
        for (backend_name, engine) in [
            ("python-psycopg3", "postgresql"),
            ("python-asyncpg", "postgresql"),
            ("python-aiomysql", "mysql"),
            ("python-aiosqlite", "sqlite"),
            ("python-duckdb", "duckdb"),
            ("python-oracledb", "oracle"),
            ("python-pyodbc", "mssql"),
            ("python-snowflake", "snowflake"),
        ] {
            let backend = crate::get_backend(backend_name, engine)
                .unwrap_or_else(|e| panic!("{backend_name} with {engine}: {e}"));
            let line = header_line(
                backend.as_ref(),
                "0.13.0",
                engine,
                "sch1:0123456789abcdef",
                "q1:fedcba9876543210",
            );
            let line = line.strip_suffix('\n').expect("header line ends with a newline");

            assert!(
                line.len() > RUFF_DEFAULT_LINE_LENGTH,
                "test premise: {backend_name}'s header is expected to exceed ruff's default \
                 line length, so the suppression is genuinely required; got {} chars: {line:?}",
                line.len()
            );
            assert!(
                line.ends_with("  # noqa: E501"),
                "{backend_name}: a header longer than ruff's default {RUFF_DEFAULT_LINE_LENGTH} \
                 columns must carry an E501 suppression, or every generated file fails \
                 `ruff check --select E` on line 1: {line:?}"
            );
        }
    }

    /// The suppression must not leak into a language that shares Python's
    /// `#` comment token. Ruby is the case that would actually ship broken:
    /// its header is emitted into `queries.rb` *and* `queries.rbs`.
    #[test]
    fn hash_comment_languages_other_than_python_get_no_suffix() {
        let backend = crate::get_backend("ruby-pg", "postgresql").expect("ruby-pg should support postgresql");
        let line = header_line(
            backend.as_ref(),
            "0.13.0",
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        );

        assert_eq!(
            line,
            "# scythe:provenance v=0.13.0 backend=ruby-pg engine=postgresql schema=sch1:0123456789abcdef \
             queries=q1:fedcba9876543210\n"
        );
    }

    #[test]
    fn sanitize_field_strips_newline_and_carriage_return() {
        assert_eq!(sanitize_field("postgresql\nfn evil() {}"), "postgresqlfn evil() {}");
        assert_eq!(sanitize_field("postgresql\r\nfn evil() {}"), "postgresqlfn evil() {}");
        assert_eq!(sanitize_field("postgresql\rfn evil() {}"), "postgresqlfn evil() {}");
        assert_eq!(sanitize_field("clean"), "clean");
    }

    /// Documents why the verifier's sanitized-vs-sanitized SC-PRV04
    /// comparison cannot be exercised end to end with a value that actually
    /// differs pre/post sanitization: `get_backend` rejects any `engine`
    /// whose `normalize_engine()` output is not an exact-string match for
    /// one of the backend's `supported_engines()`, and every recognized
    /// alias is a clean literal containing neither `\n` nor `\r`. Any engine
    /// value containing one fails `get_backend` before verification ever
    /// reaches the comparison — so the sanitizing comparison closes a latent
    /// bug, not a currently reachable one. It is still correct:
    /// [`header_line`] accepts an arbitrary `&str` with no such gate, so the
    /// comparison must not depend on `get_backend` policing its input for it.
    #[test]
    fn sanitize_field_is_a_no_op_for_every_alias_get_backend_accepts() {
        for alias in [
            "postgresql",
            "postgres",
            "pg",
            "cockroachdb",
            "crdb",
            "mysql",
            "mariadb",
            "sqlite",
            "sqlite3",
            "duckdb",
            "mssql",
            "sqlserver",
            "tsql",
            "oracle",
            "snowflake",
            "redshift",
        ] {
            assert_eq!(sanitize_field(alias).as_ref(), alias);
        }
    }

    #[test]
    fn header_line_embeds_the_caller_supplied_version_not_this_crates_own() {
        let backend = crate::get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let line = header_line(
            backend.as_ref(),
            "9.9.9-caller",
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        );

        assert_eq!(
            line,
            "// scythe:provenance v=9.9.9-caller backend=rust-sqlx engine=postgresql schema=sch1:0123456789abcdef \
             queries=q1:fedcba9876543210\n"
        );
    }

    #[test]
    fn assemble_file_omits_separator_when_preamble_is_empty() {
        assert_eq!(assemble_file("", "// header\n", "body\n"), "// header\nbody\n");
    }

    #[test]
    fn assemble_file_inserts_separator_when_preamble_is_non_empty() {
        assert_eq!(
            assemble_file("<?php\n", "// header\n", "declare(strict_types=1);\n"),
            "<?php\n// header\n\ndeclare(strict_types=1);\n"
        );
    }
}
