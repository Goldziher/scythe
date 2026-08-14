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
//!
//! The same argument is why the *reader* half — [`sanitize_field`]'s
//! inverse [`decode_field`], and the [`parse_header_fields`] tokenizer that
//! applies it — sits here beside [`header_line`] rather than beside the
//! verifier that consumes it. A writer and a reader of one format are two
//! derivations of the same fact; kept apart, each can only be tested
//! against a hand-written expected string, and both tests keep passing
//! while the pair drifts. Kept together, one test asserts the round trip.

use std::borrow::Cow;
use std::fmt::Write as _;

use crate::backend_trait::CodegenBackend;

/// The token every provenance header line is built around, and the anchor a
/// reader locates the `key=value` tail from.
///
/// Deliberately comment-syntax-agnostic: the header sits behind whatever
/// comment token the target language uses (`//`, `#`, `--`, a block
/// comment, ...), so a reader finds this substring and treats everything
/// after it on that line as the field list, instead of needing a
/// per-language rule for stripping each comment syntax.
pub const HEADER_SENTINEL: &str = "scythe:provenance";

/// Line-comment token to embed the provenance header behind, derived from
/// `manifest().backend.language`.
///
/// `language` is a `String`, not a Rust enum, but exactly 10 distinct values
/// are used across every one of the 102 shipped manifests, so this one match
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
/// The suffix is invisible to a reader: [`parse_header_fields`] tokenizes
/// the text after the sentinel on whitespace and skips any token without an
/// `=`, so `#`, `noqa:`, and `E501` are all ignored and the five
/// `key=value` fields still parse. Pinned by
/// `parse_header_fields_skips_the_python_noqa_suffix` below, and by
/// `assemble_output_python_header_carries_noqa_and_still_round_trips` in
/// `scythe-cli`.
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

/// True for the characters [`sanitize_field`] must escape: the escape
/// introducer itself, plus everything that would break the two structural
/// guarantees the header format rests on.
///
/// - **Whitespace** would break field framing. The header tail is a
///   whitespace-separated list of `key=value` tokens, so a raw space inside
///   a value splits that value into a token with no `=` — which a reader
///   drops on the floor, silently truncating the field (#133). `is_control`
///   and `is_whitespace` together cover more than ASCII space: `\n` and `\r`
///   would terminate the comment outright (below), `\t`/`\x0b`/`\x0c` split
///   under `split_ascii_whitespace`, and a *Unicode* space such as U+00A0
///   at the very end of the last value is eaten by the `str::trim` a reader
///   applies to the tail. All of them are escaped so no reader has to care
///   which class a given character falls in.
/// - **Line terminators** would break the "the header is always a comment"
///   guarantee. A value containing `\n` or a lone `\r` ends the comment
///   early: everything after it lands on its own physical line with no
///   comment prefix, becoming live, uncommented content in the generated
///   file.
fn needs_escape(character: char) -> bool {
    character == '\\' || character.is_whitespace() || character.is_control()
}

/// Encode a provenance field value so it can be embedded in the header line
/// and read back byte-for-byte. The exact inverse of [`decode_field`].
///
/// Escapes with a backslash: `\` becomes `\\`, and any character
/// [`needs_escape`] flags becomes `\u{<hex>}` (e.g. a space becomes
/// `\u{20}`). The result therefore contains no whitespace and no control
/// characters at all, which is what makes the header a well-framed,
/// single-line, always-a-comment token list regardless of what a caller
/// passes in.
///
/// In practice this is the identity function. Every value that reaches
/// [`header_line`] today — a semver `version`, a hardcoded `backend.name()`
/// literal, an `engine` alias, `sch1:` + 16 hex, `q1:` + 16 hex — contains
/// none of the escaped characters, so no shipped header's bytes change and
/// no `Cow` is ever allocated. The escaping exists for the one field that
/// is *not* constrained: `engine` is the `[[sql]]` `engine = "..."` value,
/// deserialized verbatim from the user's `scythe.toml` with no validation
/// upstream (`normalize_engine` only consults it to pick a dialect; the raw
/// string is what a caller passes through to here). Encoding at the point
/// of embedding, rather than at config parse time, means the guarantee
/// holds regardless of how a value arrives — this call site today, or any
/// future one — not just for callers that happen to validate first.
///
/// The verifier (`scythe check`) must encode its configured engine the same
/// way before comparing it against the header's `engine` field: the header
/// always holds the encoded value, so comparing it against a raw one would
/// permanently false-flag SC-PRV04 for any config whose engine string
/// needed escaping. Comparing encoded-to-encoded is equivalent to comparing
/// decoded-to-decoded, because the encoding is injective.
pub fn sanitize_field(value: &str) -> Cow<'_, str> {
    if !value.contains(needs_escape) {
        return Cow::Borrowed(value);
    }

    let mut encoded = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => encoded.push_str(r"\\"),
            character if needs_escape(character) => {
                write!(encoded, "\\u{{{:x}}}", character as u32).expect("writing to a String cannot fail");
            }
            character => encoded.push(character),
        }
    }
    Cow::Owned(encoded)
}

/// Decode a `u{<hex>}` escape body sitting immediately after a backslash,
/// returning the character it denotes and the input remaining after the
/// closing brace. `None` if `after_backslash` does not open a well-formed,
/// in-range escape.
fn decode_unicode_escape(after_backslash: &str) -> Option<(char, &str)> {
    let body = after_backslash.strip_prefix("u{")?;
    let close = body.find('}')?;
    let code_point = u32::from_str_radix(&body[..close], 16).ok()?;
    Some((char::from_u32(code_point)?, &body[close + 1..]))
}

/// Recover the original field value from what [`sanitize_field`] embedded.
/// The exact inverse of that function — the round trip is pinned by
/// `header_line_round_trips_adversarial_field_values` below.
///
/// Lenient by design, which is what keeps every header committed before
/// this encoding existed readable: a backslash that does not introduce a
/// recognized escape decodes to a literal backslash rather than an error.
/// Old-format headers are unaffected in a stronger sense than "tolerated" —
/// their values (semver, backend literal, engine alias, `sch1:`/`q1:` plus
/// hex) contain no backslash at all, so decoding them is bit-for-bit the
/// identity. Rejecting a lone backslash instead would buy nothing: there is
/// no format-version field to switch on, so a stricter reader could only
/// turn an old file into a hard error where today it reads correctly.
pub fn decode_field(value: &str) -> Cow<'_, str> {
    if !value.contains('\\') {
        return Cow::Borrowed(value);
    }

    let mut decoded = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(backslash) = rest.find('\\') {
        decoded.push_str(&rest[..backslash]);
        let after_backslash = &rest[backslash + 1..];

        if let Some(tail) = after_backslash.strip_prefix('\\') {
            decoded.push('\\');
            rest = tail;
        } else if let Some((character, tail)) = decode_unicode_escape(after_backslash) {
            decoded.push(character);
            rest = tail;
        } else {
            decoded.push('\\');
            rest = after_backslash;
        }
    }
    decoded.push_str(rest);
    Cow::Owned(decoded)
}

/// Read back the `key=value` fields [`header_line`] wrote, decoded.
///
/// This is the parsing half of the header format, and it lives here rather
/// than beside its caller for the same reason the rest of this module does:
/// the emitted line and the reader that consumes it are two derivations of
/// one fact, and a private second copy of the tokenizer would drift from
/// the writer silently — in exactly the case that matters. Keeping the pair
/// adjacent is what lets one test assert the round trip
/// (`header_line_round_trips_adversarial_field_values`) instead of two
/// tests each asserting half of it against a hand-written string.
///
/// `scythe check` still reads headers through its own private tokenizer in
/// `scythe-cli`, which does not decode. That stays correct rather than
/// merely tolerable: it compares the field it read against the *encoded*
/// configured value (it runs that value through [`sanitize_field`] first),
/// and comparing encoded-to-encoded is equivalent to comparing the decoded
/// values because the encoding is injective. Adopting this function there
/// buys better wording in a drift message, not different verdicts.
///
/// Returns `None` when `line` carries no [`HEADER_SENTINEL`] at all. Keys
/// are returned in the order they appear, borrowed from `line`; values are
/// [`decode_field`]-decoded. Tokens with no `=` are skipped, which is what
/// makes the per-language [`header_suffix`] (Python's `# noqa: E501`)
/// invisible to a reader. Unknown keys are returned rather than rejected:
/// deciding which keys matter belongs to the caller, so the header format
/// and any given verifier can grow independently.
pub fn parse_header_fields(line: &str) -> Option<Vec<(&str, String)>> {
    let sentinel_start = line.find(HEADER_SENTINEL)?;
    let tail = line[sentinel_start + HEADER_SENTINEL.len()..].trim();

    Some(
        tail.split_ascii_whitespace()
            .filter_map(|token| token.split_once('='))
            .map(|(key, value)| (key, decode_field(value).into_owned()))
            .collect(),
    )
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
/// `queries` is the `q1:<16 hex>` fingerprint of the analyzed query set that
/// produced this file (see `AnalyzedQuery::fingerprint_set` in
/// `scythe-core`), the `queries=` counterpart to `schema`'s `sch1:<16 hex>`
/// — added in #94 so that editing a `.sql` query file without touching the
/// schema is no longer invisible to `scythe check`.
///
/// *Every* field is passed through [`sanitize_field`] before embedding, not
/// just the free-form `engine` one. Four of the five cannot contain an
/// escapable character today, so encoding them is provably a no-op and the
/// emitted bytes are unchanged — but "cannot" is a property of the current
/// callers, not of this function's signature, which accepts an arbitrary
/// `&str` for each. Encoding uniformly means [`parse_header_fields`] is the
/// exact inverse of this function for *any* input, which is what the
/// round-trip test can assert; encoding selectively would make the inverse
/// hold only for the argument positions someone remembered to cover.
///
/// A per-language [`header_suffix`] may be appended after the last field —
/// today only Python's `# noqa: E501`, without which the line trips ruff's
/// default 88-character limit in every generated file.
pub fn header_line(backend: &dyn CodegenBackend, version: &str, engine: &str, schema: &str, queries: &str) -> String {
    build_header_line(backend, version, engine, schema, queries, None)
}

/// [`header_line`]'s sibling for the `options=` sixth field (GH #155):
/// a fingerprint of a `[[sql.gen]]` target's resolved options (derive list,
/// serde flag, `row_type`, naming case, ...) and, when one is configured,
/// its manifest overlay's contents -- see [`options_fingerprint`], which
/// produces the `options` value this function embeds.
///
/// A separate function rather than a new required parameter on
/// [`header_line`] deliberately: dozens of call sites across
/// `scythe-codegen`'s own regression tests construct a header with the
/// original five-argument form to build an *expected* string for structural
/// validation, not to assert on the header's exact bytes, and those files
/// are generated by `tools/integration-test-generator` -- not hand-edited,
/// so their call sites cannot be updated here. Changing `header_line`'s
/// arity would have been a breaking change this crate cannot fix at every
/// call site. Delegating both functions to the same private
/// [`build_header_line`] keeps the five-field and six-field forms unable to
/// drift from each other on everything but the one field that differs.
pub fn header_line_with_options(
    backend: &dyn CodegenBackend,
    version: &str,
    engine: &str,
    schema: &str,
    queries: &str,
    options: &str,
) -> String {
    build_header_line(backend, version, engine, schema, queries, Some(options))
}

/// Shared implementation behind [`header_line`] (`options: None`) and
/// [`header_line_with_options`] (`options: Some(...)`). `options.is_none()`
/// produces bytes identical to the pre-#155 five-field header -- the
/// `options=` field, and the space before it, are only ever emitted when a
/// value is supplied -- so this refactor is behavior-preserving for every
/// existing [`header_line`] caller.
fn build_header_line(
    backend: &dyn CodegenBackend,
    version: &str,
    engine: &str,
    schema: &str,
    queries: &str,
    options: Option<&str>,
) -> String {
    let language = &backend.manifest().backend.language;
    let comment = comment_prefix(language);
    let suffix = header_suffix(language);
    let version = sanitize_field(version);
    let name = sanitize_field(backend.name());
    let engine = sanitize_field(engine);
    let schema = sanitize_field(schema);
    let queries = sanitize_field(queries);
    let options_field = match options {
        Some(value) => format!(" options={}", sanitize_field(value)),
        None => String::new(),
    };
    format!(
        "{comment} {HEADER_SENTINEL} v={version} backend={name} engine={engine} schema={schema} \
         queries={queries}{options_field}{suffix}\n"
    )
}

/// Version tag for the gen-target options fingerprint algorithm, mirroring
/// `q1`/`sch1`'s role for the query/schema fingerprints in `scythe-core`.
const OPTIONS_FINGERPRINT_ALGORITHM_TAG: &str = "opt1";

/// FNV-1a, 64-bit variant.
///
/// Chosen over a cryptographic hash so [`options_fingerprint`] costs this
/// crate no new dependency: `scythe-codegen`'s `Cargo.toml` carries none
/// today, and `scythe-core`'s SHA-256 (used for the sibling schema/query
/// fingerprints) is private to that crate. FNV-1a takes no seed and reads no
/// process or environment state -- its output is a pure function of the
/// input bytes, which is exactly the property this fingerprint needs: the
/// same option set must hash identically on every machine and every run,
/// forever. A collision only under-reports drift (treats two different
/// option sets as the same), never over-reports it, which is an acceptable
/// failure mode for a hint that `scythe generate` fixes either way -- this
/// is not a security boundary a cryptographic hash exists to defend.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET_BASIS;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// The `options=` field [`header_line_with_options`] embeds: a deterministic
/// fingerprint of a `[[sql.gen]]` target's resolved options, plus the
/// contents of its manifest overlay when one is configured (GH #155).
///
/// Before this existed, the header carried five fields -- version, backend,
/// engine, schema, queries -- and none of them moved when only an *option*
/// changed: flipping `row_type`, adding a `derive`, or swapping in a
/// different `manifest = "..."` overlay left the header byte-identical, so
/// `scythe check` reported a stale artifact as fresh.
///
/// `options` is any iterator of `(key, value)` pairs -- a caller typically
/// passes a `[[sql.gen]]` target's resolved `HashMap<String, String>` by
/// reference (`.iter().map(|(k, v)| (k.as_str(), v.as_str()))`); sorting
/// happens inside this function so the caller never has to remember to do
/// it. `manifest_overlay` is the *contents* of the overlay file when
/// `[[sql.gen]] manifest = "..."` names one, `None` otherwise --
/// fingerprinting the file's bytes, not its path, so drift is detected when
/// the overlay's *content* changes and is never spuriously reported just
/// because the same project was checked out to a different path or on a
/// different machine.
///
/// # Stability
///
/// - **Order-independent.** `options` is sorted by key before it is hashed,
///   so it does not matter whether the caller's `HashMap` (whose iteration
///   order is unspecified, and in practice randomized per process by
///   `std`'s default hasher) happened to yield its entries in one order or
///   another on a given run.
/// - **Machine- and run-independent.** [`fnv1a_64`] takes no seed and reads
///   no environment -- see that function's doc comment. This is exactly why
///   `ahash` (already a dependency of both this crate and `scythe-cli`,
///   whose `AHashSet`/`AHashMap` are used elsewhere in this codebase) was
///   deliberately *not* reused here: `ahash::AHasher::default()`'s "fixed"
///   keys are generated once per process from the OS random source (see
///   `ahash::random_state::get_fixed_seeds`, gated on the `std` feature),
///   not compiled in, so the identical option set hashes differently
///   between two separate `scythe generate`/`scythe check` invocations --
///   which would make every fingerprint here spurious drift on every run.
/// - **Injective encoding.** Every key, value, and the overlay's content are
///   each passed through [`sanitize_field`] before being folded into the
///   canonical form, so no field can inject a fake `\t`/`\n` delimiter and
///   make two differently-shaped option sets hash the same -- the same
///   anti-forgery reasoning [`header_line`] already applies to the fields it
///   embeds.
/// - **Path-independent.** Only the overlay's *content* is hashed, never its
///   path -- see above.
///
/// The result is `opt1:<16 hex>`, matching the `sch1:`/`q1:` shape of the
/// sibling schema/query fingerprints so all three read as one family.
pub fn options_fingerprint<'a, I>(options: I, manifest_overlay: Option<&str>) -> String
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut sorted: Vec<(&str, &str)> = options.into_iter().collect();
    sorted.sort_unstable_by_key(|(key, _)| *key);

    let mut canonical = String::new();
    for (key, value) in sorted {
        canonical.push_str("opt\t");
        canonical.push_str(&sanitize_field(key));
        canonical.push('\t');
        canonical.push_str(&sanitize_field(value));
        canonical.push('\n');
    }
    if let Some(overlay) = manifest_overlay {
        canonical.push_str("manifest\t");
        canonical.push_str(&sanitize_field(overlay));
        canonical.push('\n');
    }

    let hash = fnv1a_64(canonical.as_bytes());
    format!("{OPTIONS_FINGERPRINT_ALGORITHM_TAG}:{hash:016x}")
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
    fn sanitize_field_escapes_newline_carriage_return_and_space() {
        assert_eq!(
            sanitize_field("postgresql\nfn evil() {}"),
            r"postgresql\u{a}fn\u{20}evil()\u{20}{}"
        );
        assert_eq!(
            sanitize_field("postgresql\r\nfn evil() {}"),
            r"postgresql\u{d}\u{a}fn\u{20}evil()\u{20}{}"
        );
        assert_eq!(
            sanitize_field("postgresql\rfn evil() {}"),
            r"postgresql\u{d}fn\u{20}evil()\u{20}{}"
        );
        assert_eq!(sanitize_field("clean"), "clean");
    }

    /// The encoded form must contain nothing that can split a token or end
    /// the comment — that is the entire structural contract the header
    /// format rests on, and it has to hold for characters no test thought
    /// to enumerate, not just for `\n`, `\r`, and space.
    #[test]
    fn sanitize_field_output_never_contains_whitespace_or_control_characters() {
        for value in ADVERSARIAL_FIELD_VALUES {
            let encoded = sanitize_field(value);
            assert!(
                !encoded.contains(|c: char| c.is_whitespace() || c.is_control()),
                "encoding {value:?} left whitespace or a control character in {encoded:?}"
            );
        }
    }

    /// A backslash is the escape introducer, so it must itself be escaped —
    /// otherwise a value ending in `\` would swallow the `u{...}` of a
    /// following escape and decode to something else entirely.
    #[test]
    fn sanitize_field_escapes_the_escape_introducer() {
        assert_eq!(sanitize_field(r"c:\tmp"), r"c:\\tmp");
        assert_eq!(sanitize_field(r"\u{20}"), r"\\u{20}");
        assert_eq!(decode_field(&sanitize_field(r"\u{20}")), r"\u{20}");
    }

    /// Headers committed before this encoding existed carry raw values, and
    /// they must keep reading exactly as they always did. They do so for a
    /// stronger reason than tolerance: no legal old value contains a
    /// backslash, so decoding is bit-for-bit the identity. Pinned with the
    /// real shapes — semver, canonical backend name, engine alias, and the
    /// two fingerprints.
    #[test]
    fn decode_field_is_the_identity_for_old_format_header_values() {
        for value in [
            "0.13.0",
            "0.15.0-rc.1",
            "rust-sqlx",
            "python-psycopg3",
            "postgresql",
            "mariadb",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        ] {
            assert_eq!(decode_field(value).as_ref(), value);
            assert!(matches!(decode_field(value), Cow::Borrowed(_)), "{value:?} reallocated");
        }
    }

    /// A backslash that opens no recognized escape decodes to a literal
    /// backslash rather than an error or a dropped character — the leniency
    /// that keeps an unknown or hand-edited header readable.
    #[test]
    fn decode_field_treats_an_unrecognized_escape_as_a_literal_backslash() {
        assert_eq!(decode_field(r"c:\tmp"), r"c:\tmp");
        assert_eq!(decode_field(r"trailing\"), "trailing\\");
        assert_eq!(decode_field(r"\u{}"), r"\u{}");
        assert_eq!(decode_field(r"\u{zz}"), r"\u{zz}");
        assert_eq!(decode_field(r"\u{d800}"), r"\u{d800}", "lone surrogate is not a char");
        assert_eq!(decode_field(r"\u{20"), r"\u{20", "unterminated escape");
    }

    /// Field values chosen to break the header format in every way it can
    /// be broken: token framing (space, tab, the Unicode space a `trim`
    /// eats), the "always a comment" guarantee (`\n`, `\r`), the `key=value`
    /// split (`=`), the escape scheme itself (backslashes, a literal
    /// `\u{...}`), the sentinel search, and the degenerate empty value.
    const ADVERSARIAL_FIELD_VALUES: &[&str] = &[
        "",
        "postgresql",
        "sql/my schema.sql",
        "a b c",
        " leading",
        "trailing ",
        "\ttab\t",
        "line\nbreak",
        "carriage\rreturn",
        "crlf\r\nboth",
        "nbsp\u{a0}space",
        "vertical\u{b}tab",
        "key=value=pairs",
        "=starts-with-equals",
        "no-equals-at-all",
        r"back\slash",
        r"\\double",
        r"\u{20}literal-escape",
        "\"quoted\"",
        "'single'",
        "scythe:provenance v=9.9.9",
        "// scythe:provenance",
        "# noqa: E501",
        "ünïcödé-ロケール",
        "emoji-🎉-field",
        "\u{0}nul",
        "brace}close",
    ];

    /// The invariant this module exists to hold: whatever a caller puts in,
    /// a reader gets back, byte for byte, for every field. [`header_line`]
    /// and [`parse_header_fields`] are two derivations of one format, and
    /// this is the only test that can catch them disagreeing — a test that
    /// checked either half against a hand-written expected string would
    /// pass happily while the pair drifted.
    ///
    /// Also asserts the two structural guarantees a caller cannot recover
    /// from if they are lost: the header stays exactly one line, and it
    /// stays behind a comment token.
    #[test]
    fn header_line_round_trips_adversarial_field_values() {
        // Two backends, because the emitted line differs by language:
        // `python-psycopg3` appends `# noqa: E501` after the last field,
        // which the reader must skip rather than read as a field.
        for backend_name in ["rust-sqlx", "python-psycopg3"] {
            let backend = crate::get_backend(backend_name, "postgresql")
                .unwrap_or_else(|e| panic!("{backend_name} with postgresql: {e}"));
            let comment = comment_prefix(&backend.manifest().backend.language);

            for version in ADVERSARIAL_FIELD_VALUES {
                for other in ADVERSARIAL_FIELD_VALUES {
                    let line = header_line(backend.as_ref(), version, other, version, other);

                    assert_eq!(
                        line.lines().count(),
                        1,
                        "{backend_name}: header must stay one line for {version:?}/{other:?}, got {line:?}"
                    );
                    assert!(
                        line.starts_with(comment),
                        "{backend_name}: header must stay a comment, got {line:?}"
                    );

                    let fields =
                        parse_header_fields(&line).unwrap_or_else(|| panic!("{backend_name}: no sentinel in {line:?}"));
                    let read = |key: &str| {
                        fields
                            .iter()
                            .find(|(k, _)| *k == key)
                            .map(|(_, v)| v.as_str())
                            .unwrap_or_else(|| panic!("{backend_name}: no {key}= in {line:?}"))
                    };

                    assert_eq!(read("v"), *version, "version round trip, line: {line:?}");
                    assert_eq!(read("backend"), backend.name(), "backend round trip, line: {line:?}");
                    assert_eq!(read("engine"), *other, "engine round trip, line: {line:?}");
                    assert_eq!(read("schema"), *version, "schema round trip, line: {line:?}");
                    assert_eq!(read("queries"), *other, "queries round trip, line: {line:?}");
                }
            }
        }
    }

    /// The round trip must also survive assembly, where the header is one
    /// line among many rather than the whole string — the shape a verifier
    /// actually reads off disk.
    #[test]
    fn assembled_file_round_trips_a_space_bearing_field_value() {
        let backend = crate::get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let engine = "postgresql\nfn evil() {}";

        let file = assemble_file(
            "",
            &header_line(
                backend.as_ref(),
                "0.15.0",
                engine,
                "sch1:0123456789abcdef",
                "q1:fedcba9876543210",
            ),
            "pub fn generated() {}\n",
        );

        assert!(
            !file.lines().any(|line| line.trim_start().starts_with("fn evil()")),
            "injected content must never appear as its own uncommented line:\n{file}"
        );

        let header = file
            .lines()
            .find(|line| line.contains(HEADER_SENTINEL))
            .expect("assembled file must carry a header line");
        let fields = parse_header_fields(header).expect("header line must parse");
        let parsed_engine = fields
            .iter()
            .find(|(key, _)| *key == "engine")
            .map(|(_, value)| value.as_str());

        assert_eq!(parsed_engine, Some(engine), "header: {header:?}");
    }

    /// Tokens with no `=` must be skipped, or Python's `# noqa: E501`
    /// suffix would be read as fields. Asserted on the real emitted line
    /// rather than a hand-written one.
    #[test]
    fn parse_header_fields_skips_the_python_noqa_suffix() {
        let backend =
            crate::get_backend("python-psycopg3", "postgresql").expect("python-psycopg3 should support postgresql");
        let line = header_line(
            backend.as_ref(),
            "0.15.0",
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        );

        let keys: Vec<&str> = parse_header_fields(&line)
            .expect("header line must parse")
            .into_iter()
            .map(|(key, _)| key)
            .collect();

        assert_eq!(keys, vec!["v", "backend", "engine", "schema", "queries"]);
    }

    #[test]
    fn parse_header_fields_returns_none_without_the_sentinel() {
        assert!(parse_header_fields("// just a comment v=1.2.3").is_none());
    }

    /// Unknown keys are returned, not dropped: the header format and any
    /// given reader have to be able to grow independently.
    #[test]
    fn parse_header_fields_preserves_unknown_keys() {
        let fields =
            parse_header_fields("// scythe:provenance v=0.15.0 future_field=xyz").expect("header line must parse");
        assert_eq!(
            fields,
            vec![("v", "0.15.0".to_string()), ("future_field", "xyz".to_string())]
        );
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

    // -----------------------------------------------------------------------
    // `header_line_with_options` (GH #155)
    // -----------------------------------------------------------------------

    /// The refactor behind `header_line_with_options` (splitting `header_line`
    /// into a shared `build_header_line`) must not move a single byte of the
    /// five-field header every existing caller -- including the
    /// generator-produced regression suites this crate cannot hand-edit --
    /// still constructs. Pinned as a literal, not compared against a second
    /// call to `header_line`, so a change to `build_header_line` that moved
    /// *both* callers together the same wrong way would still be caught.
    #[test]
    fn header_line_is_byte_identical_to_the_pre_options_five_field_form() {
        let backend = crate::get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let line = header_line(
            backend.as_ref(),
            "0.15.0",
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        );

        assert_eq!(
            line,
            "// scythe:provenance v=0.15.0 backend=rust-sqlx engine=postgresql schema=sch1:0123456789abcdef \
             queries=q1:fedcba9876543210\n"
        );
    }

    #[test]
    fn header_line_with_options_appends_the_sixth_field_before_the_suffix() {
        let backend = crate::get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let line = header_line_with_options(
            backend.as_ref(),
            "0.15.0",
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
            "opt1:0011223344556677",
        );

        assert_eq!(
            line,
            "// scythe:provenance v=0.15.0 backend=rust-sqlx engine=postgresql schema=sch1:0123456789abcdef \
             queries=q1:fedcba9876543210 options=opt1:0011223344556677\n"
        );
    }

    /// The suffix (Python's `# noqa: E501`) must stay the last thing on the
    /// line, after `options=`, exactly as it already stays after `queries=`.
    #[test]
    fn header_line_with_options_keeps_the_python_suffix_last() {
        let backend =
            crate::get_backend("python-psycopg3", "postgresql").expect("python-psycopg3 should support postgresql");
        let line = header_line_with_options(
            backend.as_ref(),
            "0.15.0",
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
            "opt1:0011223344556677",
        );

        assert!(
            line.ends_with("options=opt1:0011223344556677  # noqa: E501\n"),
            "got: {line:?}"
        );
    }

    #[test]
    fn header_line_with_options_round_trips_through_parse_header_fields() {
        let backend = crate::get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let line = header_line_with_options(
            backend.as_ref(),
            "0.15.0",
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
            "opt1:0011223344556677",
        );

        let fields = parse_header_fields(&line).expect("header line must parse");
        let options = fields
            .iter()
            .find(|(key, _)| *key == "options")
            .map(|(_, value)| value.as_str());

        assert_eq!(options, Some("opt1:0011223344556677"));
    }

    // -----------------------------------------------------------------------
    // `options_fingerprint` (GH #155)
    // -----------------------------------------------------------------------

    #[test]
    fn options_fingerprint_has_the_opt1_tag_shape() {
        let fp = options_fingerprint([("row_type", "pydantic")], None);
        assert!(fp.starts_with("opt1:"), "got: {fp:?}");
        assert_eq!(
            fp.len(),
            "opt1:".len() + 16,
            "expected a 16-hex-digit digest after the tag, got: {fp:?}"
        );
    }

    /// The whole reason `options_fingerprint` sorts internally: a caller's
    /// `HashMap` iterates in an order this function must not be sensitive
    /// to, since two runs of `scythe generate` against the identical config
    /// can iterate that map differently.
    #[test]
    fn options_fingerprint_is_independent_of_input_order() {
        let forward = options_fingerprint([("derive", "Debug"), ("row_type", "pydantic"), ("serde", "true")], None);
        let backward = options_fingerprint([("serde", "true"), ("row_type", "pydantic"), ("derive", "Debug")], None);

        assert_eq!(forward, backward);
    }

    /// Calling the function twice with the identical input must produce the
    /// identical output -- the property that makes it safe to call once when
    /// writing the header and again, independently, when verifying it.
    #[test]
    fn options_fingerprint_is_deterministic_across_repeated_calls() {
        let a = options_fingerprint(
            [("row_type", "pydantic")],
            Some("[naming]\nstruct_case = \"PascalCase\"\n"),
        );
        let b = options_fingerprint(
            [("row_type", "pydantic")],
            Some("[naming]\nstruct_case = \"PascalCase\"\n"),
        );

        assert_eq!(a, b);
    }

    #[test]
    fn options_fingerprint_changes_when_an_option_value_changes() {
        let pydantic = options_fingerprint([("row_type", "pydantic")], None);
        let dataclass = options_fingerprint([("row_type", "dataclass")], None);

        assert_ne!(pydantic, dataclass, "a changed option value must move the fingerprint");
    }

    #[test]
    fn options_fingerprint_changes_when_an_option_is_added_or_removed() {
        let one_option = options_fingerprint([("row_type", "pydantic")], None);
        let two_options = options_fingerprint([("row_type", "pydantic"), ("serde", "true")], None);

        assert_ne!(one_option, two_options, "an added option must move the fingerprint");
    }

    #[test]
    fn options_fingerprint_changes_when_the_manifest_overlay_gains_content() {
        let no_overlay = options_fingerprint([("row_type", "pydantic")], None);
        let with_overlay = options_fingerprint(
            [("row_type", "pydantic")],
            Some("[naming]\nstruct_case = \"snake_case\"\n"),
        );

        assert_ne!(
            no_overlay, with_overlay,
            "configuring a manifest overlay must move the fingerprint even when `options` is unchanged"
        );
    }

    /// The overlay is fingerprinted by *content*, not merely by "present or
    /// absent" -- editing the overlay file (e.g. changing a scalar mapping)
    /// must be visible even though `target.options` never changes and the
    /// overlay is configured both before and after the edit.
    #[test]
    fn options_fingerprint_changes_when_the_manifest_overlay_content_changes() {
        let before = options_fingerprint(
            [("row_type", "pydantic")],
            Some("[naming]\nstruct_case = \"PascalCase\"\n"),
        );
        let after = options_fingerprint(
            [("row_type", "pydantic")],
            Some("[naming]\nstruct_case = \"snake_case\"\n"),
        );

        assert_ne!(before, after);
    }

    #[test]
    fn options_fingerprint_of_no_options_and_no_overlay_is_deterministic() {
        assert_eq!(
            options_fingerprint(std::iter::empty(), None),
            options_fingerprint(std::iter::empty(), None)
        );
    }

    /// A key or value containing the canonical form's own delimiters
    /// (`\t`, `\n`) must not be able to forge what looks like a second
    /// `opt` or `manifest` line -- the same anti-forgery guarantee
    /// `sanitize_field` already gives every other provenance field.
    #[test]
    fn options_fingerprint_is_not_forgeable_by_a_value_containing_a_delimiter() {
        let real_two_options = options_fingerprint([("a", "1"), ("b", "2")], None);
        let forged_single_option = options_fingerprint([("a", "1\nopt\tb\t2")], None);

        assert_ne!(
            real_two_options, forged_single_option,
            "a value containing a raw newline/tab must not be able to forge an extra `opt` line"
        );
    }
}
