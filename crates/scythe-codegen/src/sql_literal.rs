//! Escape SQL text for safe splicing into a target-language string literal.
//!
//! Every backend embeds the query's SQL text directly into the generated source as a
//! string literal (`conn.prepare("SELECT ...")`, `sqlx::query!("SELECT ...")`, and so on).
//! Before this module existed, only the TypeScript backends escaped that text
//! ([`escape_ts_template_literal`](crate::backends::typescript_common::escape_ts_template_literal) and
//! [`escape_ts_double_quoted_literal`](crate::backends::typescript_common::escape_ts_double_quoted_literal)); every
//! other backend spliced the cleaned SQL straight into a host literal with no escaping at
//! all. Two distinct failure classes follow from that:
//!
//! - **Injection.** Kotlin's non-raw `"..."` string, Ruby's `"..."`, and Elixir's `"..."`
//!   are all *interpolating* literals: `$name`, `#{name}`, and `#{name}` respectively splice
//!   a same-scope variable's runtime value into the string. When SQL text contains one of
//!   those sequences and it happens to name an in-scope parameter, the generated code
//!   compiles cleanly and substitutes caller-controlled data directly into the SQL text --
//!   bypassing the `?`/`$N` parameter binding entirely. This is the root cause of the
//!   Kotlin injection tracked as issue #176.
//! - **Silent corruption or a hard compile break.** The remaining host forms (Go, Java, C#,
//!   PHP, Python, Rust, and the raw-string variants) do not interpolate, but an unescaped
//!   backslash or embedded quote either changes what the database receives (a `LIKE
//!   'a\_b%'` backslash silently becomes a real escape) or breaks the literal outright (a
//!   quoted SQL identifier like `SELECT "type"` terminates the host string early).
//!
//! One function per **host literal form** lives here, not one per backend, because the
//! literal form -- not the driver -- determines what needs escaping. Every function takes
//! the already-cleaned SQL text (post `clean_sql`/`clean_sql_oneline`/placeholder rewriting)
//! and returns text safe to splice verbatim between that host form's delimiters. None of
//! them touch delimiters themselves except [`rust_raw_string_literal`], which must choose
//! its own delimiter width.
//!
//! Only literal forms actually emitted by a backend in this crate are implemented here --
//! see each function's doc comment for which backends call it.

/// Escape SQL text for splicing into a Kotlin non-raw `"..."` string literal.
///
/// Kotlin non-raw strings interpolate on `$identifier` and `${expression}`; left
/// unescaped, SQL text containing `$name` where `name` matches an in-scope parameter
/// compiles cleanly and substitutes the parameter's runtime value directly into the SQL
/// text, bypassing the `?` binding entirely (issue #176). `\$` is Kotlin's own escape for a
/// literal dollar sign, so escaping it here is not a workaround -- it is how Kotlin spells
/// "a dollar sign that is not a template".
///
/// Also escapes backslash, the closing quote, and the three control characters
/// (newline, carriage return, tab) a non-raw Kotlin string cannot contain unescaped.
/// Used by `kotlin-jdbc`, `kotlin-r2dbc`, and `kotlin-exposed`.
pub fn escape_kotlin_string(sql: &str) -> String {
    escape_char_by_char(sql, |ch| match ch {
        '\\' => Some("\\\\"),
        '"' => Some("\\\""),
        '$' => Some("\\$"),
        '\n' => Some("\\n"),
        '\r' => Some("\\r"),
        '\t' => Some("\\t"),
        _ => None,
    })
}

/// Escape SQL text for splicing into a Ruby double-quoted `"..."` string literal.
///
/// Ruby double-quoted strings interpolate on `#{expression}`; SQL text containing that
/// sequence would otherwise splice an in-scope variable's value into the query the same
/// way Kotlin's `$name` does. Ruby double-quoted strings freely contain literal newlines
/// and tabs, so only backslash, the closing quote, and a literal `#{` need escaping.
///
/// Used by `ruby-pg`, `ruby-sqlite3`, `ruby-mysql2`, `ruby-oci8`, `ruby-trilogy`, and
/// `ruby-tiny-tds` (on the raw SQL text, before that backend's own `#{client.escape(...)}`
/// parameter interpolations are spliced in -- see that backend for why order matters there).
pub fn escape_ruby_double_quoted(sql: &str) -> String {
    escape_interpolating_double_quoted(sql, '#')
}

/// Escape SQL text for splicing into an Elixir double-quoted `"..."` string literal.
///
/// Elixir double-quoted strings interpolate on `#{expression}`, identically to Ruby.
/// Used by `elixir-postgrex`, `elixir-ecto`, `elixir-myxql`, `elixir-exqlite`, `elixir-tds`,
/// and `elixir-jamdb`.
pub fn escape_elixir_double_quoted(sql: &str) -> String {
    escape_interpolating_double_quoted(sql, '#')
}

/// Shared implementation for host literals that interpolate on `<marker>{expression}`
/// (Ruby and Elixir both use `#{...}`). Escapes backslash, the closing double quote, and
/// the two-character interpolation opener; leaves literal newlines and tabs untouched,
/// since both hosts permit them unescaped inside a double-quoted string.
fn escape_interpolating_double_quoted(sql: &str, marker: char) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c == marker && chars.peek() == Some(&'{') => {
                out.push('\\');
                out.push(marker);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Escape SQL text for splicing into a PHP single-quoted `'...'` string literal.
///
/// PHP backends (`php-pdo`, `php-amphp`) previously spliced SQL into a *double*-quoted
/// PHP string, which interpolates on `$name` and `{$expr}` the same way Kotlin's non-raw
/// strings do -- SQL text containing `$conn` or a same-named in-scope parameter would
/// substitute that variable's runtime value into the query text. PHP single-quoted strings
/// do not interpolate at all (the only two escapes they recognize are `\\` and `\'`), so
/// switching the emitted literal to single-quoted and escaping only those two characters
/// closes the injection class without needing to hunt for every interpolation trigger.
/// Literal newlines and tabs pass through unescaped, since PHP single-quoted strings permit
/// them.
pub fn escape_php_single_quoted(sql: &str) -> String {
    escape_char_by_char(sql, |ch| match ch {
        '\\' => Some("\\\\"),
        '\'' => Some("\\'"),
        _ => None,
    })
}

/// Escape SQL text for splicing into a Java plain `"..."` string literal.
///
/// Java string literals do not interpolate, but a raw newline, backslash, or double quote
/// each either fails to compile or changes what the database receives. Used by `java-jdbc`
/// and `java-r2dbc`, both of which pre-flatten SQL to one line via `clean_sql_oneline`, but
/// the newline/tab/carriage-return escapes here make the function correct independent of
/// that upstream choice.
pub fn escape_java_string(sql: &str) -> String {
    escape_char_by_char(sql, |ch| match ch {
        '\\' => Some("\\\\"),
        '"' => Some("\\\""),
        '\n' => Some("\\n"),
        '\r' => Some("\\r"),
        '\t' => Some("\\t"),
        _ => None,
    })
}

/// Escape SQL text for splicing into a Go interpreted `"..."` string literal.
///
/// Identical escape set to [`escape_java_string`] -- Go interpreted strings share the same
/// restrictions (no raw newline, backslash and quote must be escaped). Used by `go-pgx`,
/// `go-database-sql`, `go-godror`, and `go-gosnowflake`.
pub fn escape_go_interpreted_string(sql: &str) -> String {
    escape_char_by_char(sql, |ch| match ch {
        '\\' => Some("\\\\"),
        '"' => Some("\\\""),
        '\n' => Some("\\n"),
        '\r' => Some("\\r"),
        '\t' => Some("\\t"),
        _ => None,
    })
}

/// Escape SQL text for splicing into a C# verbatim `@"..."` string literal.
///
/// The C# backends previously used a *regular* `"..."` string with no escaping at all,
/// which breaks on both an embedded quote and a backslash (`LIKE 'a\_b%'` fails to compile
/// outright, since `\_` is not a recognized regular-string escape). Verbatim strings sidestep
/// backslash entirely -- it is literal there, exactly matching what SQL text needs -- and
/// permit raw newlines, so the only escape required is doubling an embedded `"` to `""`.
/// Used by all six C# backends (`csharp-npgsql`, `csharp-sqlclient`, `csharp-mysqlconnector`,
/// `csharp-microsoft-sqlite`, `csharp-oracle`, `csharp-snowflake`).
pub fn escape_csharp_verbatim_string(sql: &str) -> String {
    sql.replace('"', "\"\"")
}

/// Escape SQL text for splicing into a Python triple-double-quoted `"""..."""` string
/// literal.
///
/// Triple-quoted strings permit raw newlines and tabs, so only backslash and `"` need
/// escaping; escaping every `"` (not just runs of three) also means a lone or doubled
/// quote can never accidentally combine with the literal's own delimiter. Used by
/// `python-asyncpg`, `python-psycopg3`, `python-aiosqlite`, `python-aiomysql`,
/// `python-duckdb`, and (via `python_common::write_execute_call`) `python-oracledb`,
/// `python-snowflake`, and `python-pyodbc`.
pub fn escape_python_triple_double(sql: &str) -> String {
    escape_char_by_char(sql, |ch| match ch {
        '\\' => Some("\\\\"),
        '"' => Some("\\\""),
        _ => None,
    })
}

/// Escape SQL text for splicing into a Rust plain `"..."` string literal.
///
/// Rust string literals permit raw newlines, so only backslash and `"` need escaping.
/// Used by `rust-sibyl` and `rust-sqlx` (the latter's `sqlx::query!`/`query_as!` macros
/// receive the *decoded* string value at compile time, identical to the unescaped source
/// text, so escaping here has no effect on the macro's compile-time query verification).
pub fn escape_rust_string(sql: &str) -> String {
    escape_char_by_char(sql, |ch| match ch {
        '\\' => Some("\\\\"),
        '"' => Some("\\\""),
        _ => None,
    })
}

/// Build a complete Rust raw string literal (`r#"..."#`, widened as needed) containing
/// `sql` verbatim.
///
/// Raw strings cannot escape anything -- that is the point of using one -- so a SQL body
/// containing the delimiter sequence `"#` breaks a fixed `r#"..."#` literal outright
/// (issue #179's `rust-tokio-postgres`/`rust-tiberius` case: `"#` anywhere in the query,
/// including inside an ordinary comment or string, ends the literal early and leaves the
/// rest as malformed Rust). This widens the hash run only as far as the content requires:
/// plain SQL gets the same `r#"..."#` these backends always emitted, and only a query that
/// actually contains `"#`, `"##`, ... pays for a wider delimiter.
///
/// Returns the full literal, delimiters included, ready to splice directly in place of a
/// hand-written `r#"{}"#`.
pub fn rust_raw_string_literal(sql: &str) -> String {
    let mut hashes = 1usize;
    loop {
        let marker = format!("\"{}", "#".repeat(hashes));
        if !sql.contains(&marker) {
            break;
        }
        hashes += 1;
    }
    let delim = "#".repeat(hashes);
    format!("r{delim}\"{sql}\"{delim}")
}

/// Shared implementation for host literals whose escape set is a fixed per-character
/// substitution table with no multi-character lookahead (i.e. everything except the
/// `#{`/`${`-style interpolation escapes, which need [`escape_interpolating_double_quoted`]
/// or their own bespoke pass).
///
/// `substitute` returns `Some(replacement)` when it handled `ch` itself, `None` to fall
/// through to pushing `ch` unchanged. Iterating character-by-character rather than chaining
/// `str::replace` calls sidesteps the classic bug where an earlier pass's *output*
/// accidentally matches a later pass's pattern (e.g. escaping `"` before `\` would turn `"`
/// into `\"`, and the backslash pass would then re-escape the very backslash the quote pass
/// just inserted).
fn escape_char_by_char(sql: &str, substitute: impl Fn(char) -> Option<&'static str>) -> String {
    let mut out = String::with_capacity(sql.len());
    for ch in sql.chars() {
        match substitute(ch) {
            Some(replacement) => out.push_str(replacement),
            None => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adversarial corpus shared by every round-trip test below: interpolation markers for
    /// every interpolating host, the raw-string-breaking sequence, quote/backslash pairs a
    /// naive single-pass replace gets wrong, whitespace control characters, non-ASCII text,
    /// and a full SQL fragment with a doubled-apostrophe string literal.
    fn adversarial_corpus() -> Vec<&'static str> {
        vec![
            "",
            "$var",
            "${x}",
            "#{x}",
            "`backtick`",
            "single'quote",
            "double\"quote",
            "back\\slash",
            "double\\\\backslash",
            "new\nline",
            "tab\tchar",
            "carriage\rreturn",
            "quote\"then\\backslash",
            "backslash\\then\"quote",
            "\"#",
            "\"##",
            "café",
            "日本語",
            "SELECT * FROM t WHERE name = 'O''Brien'",
            "SELECT \"type\", \"class\" FROM items WHERE note = 'a\\_b%' AND tag = '$name' AND memo = '#{x}'",
        ]
    }

    // --- Kotlin ---------------------------------------------------------

    /// Reference unescaper matching Kotlin's own string-literal grammar: `\\`, `\"`, `\$`,
    /// `\n`, `\r`, `\t` are the only escapes [`escape_kotlin_string`] ever emits.
    fn unescape_kotlin(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('\\') => out.push('\\'),
                    Some('"') => out.push('"'),
                    Some('$') => out.push('$'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some(other) => panic!("unexpected Kotlin escape \\{other}"),
                    None => panic!("dangling backslash"),
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn kotlin_escape_round_trips_over_adversarial_corpus() {
        for input in adversarial_corpus() {
            let escaped = escape_kotlin_string(input);
            assert_eq!(unescape_kotlin(&escaped), input, "input: {input:?} -> {escaped:?}");
        }
    }

    /// Direct regression test for issue #176: SQL containing `$name`, where `name` is also
    /// the emitted parameter's identifier, must not produce a Kotlin string that
    /// interpolates it. This is the exact shape of the reported injection: a literal `$`
    /// followed by an identifier equal to an in-scope variable name.
    #[test]
    fn kotlin_escape_neutralizes_the_176_injection_shape() {
        let sql = "SELECT id, name FROM users WHERE name = $1 AND data::text = 'literal-$name-marker'";
        let escaped = escape_kotlin_string(sql);
        assert!(
            !escaped.contains("$name") || escaped.contains("\\$name"),
            "a bare, unescaped $name must never reach the Kotlin literal: {escaped:?}"
        );
        assert!(
            escaped.contains("\\$name"),
            "expected \\$name (Kotlin's escape for a literal dollar sign), got: {escaped:?}"
        );
        // Round-trips back to the exact original SQL, byte for byte.
        assert_eq!(unescape_kotlin(&escaped), sql);
    }

    #[test]
    fn kotlin_dollar_digit_is_not_kotlin_template_syntax_but_is_still_escaped() {
        // $1 is never a valid Kotlin identifier start (digits cannot begin one), so Kotlin
        // itself treats it literally either way -- but escaping it is still correct and
        // keeps the function's behavior uniform regardless of what follows `$`.
        let escaped = escape_kotlin_string("WHERE id = $1");
        assert_eq!(escaped, "WHERE id = \\$1");
    }

    // --- Ruby / Elixir (#{...} interpolation) ---------------------------

    fn unescape_hash_brace_interpolated(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('\\') => out.push('\\'),
                    Some('"') => out.push('"'),
                    Some('#') => out.push('#'),
                    Some(other) => panic!("unexpected escape \\{other}"),
                    None => panic!("dangling backslash"),
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn ruby_escape_round_trips_over_adversarial_corpus() {
        for input in adversarial_corpus() {
            let escaped = escape_ruby_double_quoted(input);
            assert_eq!(
                unescape_hash_brace_interpolated(&escaped),
                input,
                "input: {input:?} -> {escaped:?}"
            );
        }
    }

    #[test]
    fn ruby_escape_neutralizes_hash_brace_interpolation() {
        let escaped = escape_ruby_double_quoted("literal-#{name}-marker");
        assert!(
            escaped.contains("\\#{"),
            "expected an escaped #{{ opener, got: {escaped:?}"
        );
        assert!(!escaped.contains("marker-#{") && escaped.starts_with("literal-\\#{"));
    }

    /// A `#` not immediately followed by `{` is not an interpolation opener and must pass
    /// through unescaped, e.g. an ordinary `#` in a comment-like SQL fragment.
    #[test]
    fn ruby_escape_leaves_bare_hash_alone() {
        assert_eq!(escape_ruby_double_quoted("a#b"), "a#b");
    }

    #[test]
    fn elixir_escape_round_trips_over_adversarial_corpus() {
        for input in adversarial_corpus() {
            let escaped = escape_elixir_double_quoted(input);
            assert_eq!(
                unescape_hash_brace_interpolated(&escaped),
                input,
                "input: {input:?} -> {escaped:?}"
            );
        }
    }

    // --- PHP --------------------------------------------------------------

    fn unescape_php_single_quoted(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('\\') => out.push('\\'),
                    Some('\'') => out.push('\''),
                    // PHP single-quoted strings pass any other backslash through literally
                    // (e.g. `\n` stays the two characters backslash-n).
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn php_escape_round_trips_over_adversarial_corpus() {
        for input in adversarial_corpus() {
            let escaped = escape_php_single_quoted(input);
            assert_eq!(
                unescape_php_single_quoted(&escaped),
                input,
                "input: {input:?} -> {escaped:?}"
            );
        }
    }

    #[test]
    fn php_single_quoted_never_interpolates_a_dollar_variable() {
        // The point of switching to single-quoted: `$conn`/`$name` must reach the literal
        // as inert text, not a variable reference. Single-quoted PHP performs no
        // interpolation at all, so no escaping of `$` is needed for that guarantee to hold
        // -- this test documents that the chosen literal form, not a `$` escape, is what
        // closes the PHP interpolation class.
        let escaped = escape_php_single_quoted("literal-$conn-marker");
        assert_eq!(escaped, "literal-$conn-marker");
    }

    // --- Java / Go (shared escape set) -------------------------------------

    fn unescape_backslash_quote_control(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('\\') => out.push('\\'),
                    Some('"') => out.push('"'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some(other) => panic!("unexpected escape \\{other}"),
                    None => panic!("dangling backslash"),
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn java_escape_round_trips_over_adversarial_corpus() {
        for input in adversarial_corpus() {
            let escaped = escape_java_string(input);
            assert_eq!(
                unescape_backslash_quote_control(&escaped),
                input,
                "input: {input:?} -> {escaped:?}"
            );
        }
    }

    #[test]
    fn go_escape_round_trips_over_adversarial_corpus() {
        for input in adversarial_corpus() {
            let escaped = escape_go_interpreted_string(input);
            assert_eq!(
                unescape_backslash_quote_control(&escaped),
                input,
                "input: {input:?} -> {escaped:?}"
            );
        }
    }

    /// Direct regression test for the "loud case" in issue #150/#179: a quoted SQL
    /// identifier must not terminate the host literal early.
    #[test]
    fn java_escape_handles_quoted_identifier() {
        let escaped = escape_java_string("SELECT \"type\", \"class\" FROM items");
        assert!(escaped.contains("\\\"type\\\""));
        assert!(
            !escaped.contains("\"type\","),
            "the raw quote must not survive unescaped"
        );
    }

    #[test]
    fn go_escape_handles_backslash_in_like_pattern() {
        let escaped = escape_go_interpreted_string("LIKE 'a\\_b%'");
        assert_eq!(escaped, "LIKE 'a\\\\_b%'");
    }

    // --- C# -----------------------------------------------------------------

    fn unescape_csharp_verbatim(s: &str) -> String {
        s.replace("\"\"", "\"")
    }

    #[test]
    fn csharp_verbatim_round_trips_over_adversarial_corpus() {
        for input in adversarial_corpus() {
            let escaped = escape_csharp_verbatim_string(input);
            assert_eq!(
                unescape_csharp_verbatim(&escaped),
                input,
                "input: {input:?} -> {escaped:?}"
            );
        }
    }

    #[test]
    fn csharp_verbatim_leaves_backslash_literal() {
        // Verbatim strings treat `\` as an ordinary character, exactly matching what a SQL
        // `LIKE 'a\_b%'` pattern needs -- no escaping required, and the round-trip test
        // above with `back\slash` confirms the value survives unchanged.
        assert_eq!(escape_csharp_verbatim_string("a\\_b"), "a\\_b");
    }

    // --- Python ---------------------------------------------------------

    #[test]
    fn python_triple_double_round_trips_over_adversarial_corpus() {
        for input in adversarial_corpus() {
            let escaped = escape_python_triple_double(input);
            assert_eq!(
                unescape_backslash_quote_control_no_nrt(&escaped),
                input,
                "input: {input:?} -> {escaped:?}"
            );
        }
    }

    /// Like [`unescape_backslash_quote_control`], but only `\\` and `\"` are escapes --
    /// [`escape_python_triple_double`] leaves real newlines/tabs untouched, so a decoder
    /// must too.
    fn unescape_backslash_quote_control_no_nrt(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('\\') => out.push('\\'),
                    Some('"') => out.push('"'),
                    Some(other) => panic!("unexpected escape \\{other}"),
                    None => panic!("dangling backslash"),
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn python_triple_double_preserves_raw_newline() {
        let escaped = escape_python_triple_double("line1\nline2");
        assert_eq!(
            escaped, "line1\nline2",
            "python triple-quoted strings permit raw newlines"
        );
    }

    // --- Rust -------------------------------------------------------------

    #[test]
    fn rust_plain_round_trips_over_adversarial_corpus() {
        for input in adversarial_corpus() {
            let escaped = escape_rust_string(input);
            assert_eq!(
                unescape_backslash_quote_control_no_nrt(&escaped),
                input,
                "input: {input:?} -> {escaped:?}"
            );
        }
    }

    /// Extracts the SQL body back out of a full `rN#"..."N#` literal built by
    /// [`rust_raw_string_literal`], for round-trip assertions.
    fn unwrap_rust_raw_string_literal(literal: &str) -> &str {
        let rest = literal.strip_prefix('r').expect("must start with r");
        let hashes = rest.chars().take_while(|&c| c == '#').count();
        let delim = &rest[..hashes];
        let opened = &rest[hashes..];
        let opened = opened.strip_prefix('"').expect("must open with a quote");
        let closer = format!("\"{delim}");
        opened
            .strip_suffix(&closer)
            .expect("must close with matching delimiter")
    }

    #[test]
    fn rust_raw_string_round_trips_over_adversarial_corpus() {
        for input in adversarial_corpus() {
            let literal = rust_raw_string_literal(input);
            assert_eq!(
                unwrap_rust_raw_string_literal(&literal),
                input,
                "input: {input:?} -> {literal:?}"
            );
        }
    }

    /// Direct regression test for the raw-string-breaking case in issue #179: `"#` anywhere
    /// in the SQL must not terminate a fixed `r#"..."#` literal early. The widened
    /// delimiter must not appear inside the SQL body itself, or the same problem recurs one
    /// level up.
    #[test]
    fn rust_raw_string_widens_delimiter_when_sql_contains_hash_quote() {
        let sql = "a\"#b";
        let literal = rust_raw_string_literal(sql);
        assert_eq!(literal, "r##\"a\"#b\"##");
        assert_eq!(unwrap_rust_raw_string_literal(&literal), sql);
    }

    #[test]
    fn rust_raw_string_plain_sql_uses_single_hash() {
        let literal = rust_raw_string_literal("SELECT 1");
        assert_eq!(literal, "r#\"SELECT 1\"#");
    }

    #[test]
    fn rust_raw_string_widens_again_for_double_hash() {
        let sql = "a\"##b";
        let literal = rust_raw_string_literal(sql);
        assert_eq!(literal, "r###\"a\"##b\"###");
        assert_eq!(unwrap_rust_raw_string_literal(&literal), sql);
    }
}
