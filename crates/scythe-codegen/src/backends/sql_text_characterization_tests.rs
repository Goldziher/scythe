//! Characterization tests for the three pure SQL-text rewriters in
//! [`super`]: [`clean_sql`], [`clean_sql_oneline`], and
//! [`rewrite_pg_placeholders`].
//!
//! These tests pin the behaviour these functions have TODAY, exactly, so that
//! the planned rewrite (threading the SQL dialect through a real tokenizer
//! instead of the current char-scanner) either preserves that behaviour or
//! shows precisely where it diverges. Every assertion compares the full
//! output string; nothing here uses `contains`, because a `contains` check
//! passes on garbage.
//!
//! Tests whose expected value documents behaviour that is *wrong* are marked
//! with a `WRONG:` comment stating what the output should be and why. Those
//! assertions still pin the current output on purpose -- a characterization
//! test that asserts an aspiration fails on day one and tells nobody
//! anything. Do not "fix" a `WRONG:` test by changing the expectation; fix
//! the function and update the test in the same change.

use super::{clean_sql, clean_sql_oneline, rewrite_pg_placeholders};

/// Placeholder formatter used by every `rewrite_pg_placeholders` test. The
/// `[P{n}]` shape is deliberately unlike any real dialect's placeholder
/// syntax so an expected string shows at a glance which characters the
/// function replaced and which it copied through.
fn mark(n: u32) -> String {
    format!("[P{n}]")
}

// ---------------------------------------------------------------------------
// clean_sql / clean_sql_oneline: degenerate input
// ---------------------------------------------------------------------------

#[test]
fn clean_sql_of_empty_input_is_empty() {
    assert_eq!(clean_sql(""), "");
    assert_eq!(clean_sql_oneline(""), "");
}

#[test]
fn clean_sql_of_whitespace_only_input_is_empty() {
    assert_eq!(clean_sql("   \n\t \n  "), "");
    assert_eq!(clean_sql_oneline("   \n\t \n  "), "");
}

#[test]
fn clean_sql_of_line_comment_only_input_is_empty() {
    assert_eq!(clean_sql("-- just a comment"), "");
    assert_eq!(clean_sql_oneline("-- just a comment"), "");
}

#[test]
fn clean_sql_of_block_comment_only_input_is_empty() {
    assert_eq!(clean_sql("/* just a comment */"), "");
    assert_eq!(clean_sql_oneline("/* just a comment */"), "");
}

#[test]
fn clean_sql_of_lone_dollar_sign_is_preserved() {
    assert_eq!(clean_sql("SELECT $ FROM t"), "SELECT $ FROM t");
}

#[test]
fn clean_sql_of_unterminated_string_literal_keeps_the_literal_text() {
    assert_eq!(clean_sql("SELECT 'abc FROM t"), "SELECT 'abc FROM t");
}

#[test]
fn clean_sql_of_unterminated_dollar_quote_keeps_the_body_text() {
    assert_eq!(clean_sql("SELECT $$abc FROM t"), "SELECT $$abc FROM t");
}

#[test]
fn clean_sql_of_unterminated_block_comment_drops_everything_after_the_open() {
    assert_eq!(clean_sql("SELECT id FROM t /* oops"), "SELECT id FROM t");
}

// ---------------------------------------------------------------------------
// clean_sql / clean_sql_oneline: whitespace, newlines, semicolons
// ---------------------------------------------------------------------------

#[test]
fn clean_sql_trims_leading_and_trailing_whitespace() {
    assert_eq!(clean_sql("   SELECT 1   "), "SELECT 1");
}

#[test]
fn clean_sql_strips_a_single_trailing_semicolon() {
    assert_eq!(clean_sql("SELECT 1;"), "SELECT 1");
}

#[test]
fn clean_sql_strips_every_trailing_semicolon_not_just_one() {
    assert_eq!(clean_sql("SELECT 1;;;"), "SELECT 1");
}

#[test]
fn clean_sql_strips_a_trailing_semicolon_followed_by_a_newline() {
    assert_eq!(clean_sql("SELECT 1;\n"), "SELECT 1");
}

#[test]
fn clean_sql_stops_stripping_semicolons_at_the_first_intervening_space() {
    // `trim_end_matches(';')` runs once, between two `trim()` calls, so a
    // space between two trailing semicolons ends the strip.
    assert_eq!(clean_sql("SELECT 1 ; ;"), "SELECT 1 ;");
}

#[test]
fn clean_sql_keeps_statement_separating_semicolons_that_are_not_trailing() {
    assert_eq!(clean_sql("SELECT 1; SELECT 2;"), "SELECT 1; SELECT 2");
}

#[test]
fn clean_sql_does_not_strip_a_semicolon_inside_a_string_literal() {
    assert_eq!(clean_sql("SELECT 'a;'"), "SELECT 'a;'");
}

#[test]
fn clean_sql_does_not_strip_a_semicolon_inside_a_dollar_quoted_body() {
    assert_eq!(clean_sql("SELECT $$body;$$"), "SELECT $$body;$$");
}

#[test]
fn clean_sql_preserves_interior_newlines_but_oneline_joins_them_with_a_space() {
    let sql = "SELECT id\nFROM users\nWHERE id = $1";
    assert_eq!(clean_sql(sql), "SELECT id\nFROM users\nWHERE id = $1");
    assert_eq!(clean_sql_oneline(sql), "SELECT id FROM users WHERE id = $1");
}

#[test]
fn clean_sql_preserves_per_line_indentation_verbatim() {
    // WRONG(ish): the doc comment on `clean_sql` claims it strips "excess
    // whitespace", but leading indentation is copied through untouched. Only
    // the whole-string ends are trimmed. Harmless for `clean_sql` (the
    // newline layout is kept anyway) but see the oneline case below.
    let sql = "SELECT *\n  FROM t\n WHERE a = $1";
    assert_eq!(clean_sql(sql), "SELECT *\n  FROM t\n WHERE a = $1");
}

#[test]
fn clean_sql_oneline_leaks_source_indentation_as_runs_of_spaces() {
    // WRONG: a one-line rendering should collapse interior whitespace runs to
    // a single space -- `SELECT * FROM t WHERE a = $1`. Instead the join
    // adds one space on top of each line's existing indentation, so the
    // embedded literal carries the source file's formatting into generated
    // code.
    let sql = "SELECT *\n  FROM t\n WHERE a = $1";
    assert_eq!(clean_sql_oneline(sql), "SELECT *   FROM t  WHERE a = $1");
}

#[test]
fn clean_sql_keeps_source_blank_lines_that_no_comment_touched() {
    let sql = "SELECT 1\n\nFROM t";
    assert_eq!(clean_sql(sql), "SELECT 1\n\nFROM t");
}

#[test]
fn clean_sql_oneline_turns_a_source_blank_line_into_a_double_space() {
    // WRONG: a blank source line should not survive into a one-line SQL
    // literal at all; the expected output is `SELECT 1 FROM t`.
    assert_eq!(clean_sql_oneline("SELECT 1\n\nFROM t"), "SELECT 1  FROM t");
}

#[test]
fn clean_sql_normalizes_crlf_to_lf() {
    let sql = "SELECT id\r\nFROM users\r\nWHERE id = $1;";
    assert_eq!(clean_sql(sql), "SELECT id\nFROM users\nWHERE id = $1");
}

#[test]
fn clean_sql_leaves_a_bare_carriage_return_in_place() {
    // WRONG: only the `\r\n` pair is normalized, so a classic-Mac lone `\r`
    // survives into the emitted SQL literal and is neither a line break for
    // `clean_sql` nor a separator for `clean_sql_oneline`. Expected: treated
    // as a line break like `\n`.
    assert_eq!(clean_sql("SELECT 1\rFROM t"), "SELECT 1\rFROM t");
    assert_eq!(clean_sql_oneline("SELECT 1\rFROM t"), "SELECT 1\rFROM t");
}

// ---------------------------------------------------------------------------
// clean_sql / clean_sql_oneline: comments
// ---------------------------------------------------------------------------

#[test]
fn clean_sql_drops_whole_lines_that_were_only_a_header_comment() {
    let sql = "-- @name GetUser\n-- @returns :one\nSELECT 1\n";
    assert_eq!(clean_sql(sql), "SELECT 1");
    assert_eq!(clean_sql_oneline(sql), "SELECT 1");
}

#[test]
fn clean_sql_drops_an_indented_comment_only_line() {
    assert_eq!(clean_sql("SELECT 1\n   -- note\nFROM t"), "SELECT 1\nFROM t");
}

#[test]
fn clean_sql_drops_a_line_a_block_comment_emptied_of_everything_but_spaces() {
    assert_eq!(clean_sql("SELECT 1\n  /* x */  \nFROM t"), "SELECT 1\nFROM t");
}

#[test]
fn clean_sql_strips_a_trailing_line_comment_but_leaves_the_space_before_it() {
    // WRONG: stripping a trailing comment should not leave the whitespace
    // that preceded it. Expected `SELECT 1\nFROM t`; the stray trailing
    // space is then doubled by the oneline join below.
    assert_eq!(clean_sql("SELECT 1 -- hi\nFROM t"), "SELECT 1 \nFROM t");
    assert_eq!(clean_sql_oneline("SELECT 1 -- hi\nFROM t"), "SELECT 1  FROM t");
}

#[test]
fn clean_sql_replaces_a_mid_line_block_comment_with_nothing_leaving_two_spaces() {
    // WRONG: removing ` /* mid */ ` should collapse to a single separating
    // space -- `SELECT 1 FROM t`.
    assert_eq!(clean_sql("SELECT 1 /* mid */ FROM t"), "SELECT 1  FROM t");
}

#[test]
fn clean_sql_drops_the_lines_a_multiline_block_comment_spans() {
    let sql = "SELECT 1\n/* line one\n   line two */\nFROM t";
    assert_eq!(clean_sql(sql), "SELECT 1\nFROM t");
}

#[test]
fn clean_sql_treats_block_comments_as_nesting() {
    let sql = "SELECT id /* outer /* inner */ still outer */ FROM t";
    assert_eq!(clean_sql(sql), "SELECT id  FROM t");
}

#[test]
fn clean_sql_lets_an_unbalanced_nested_block_comment_swallow_the_whole_query() {
    // WRONG for every non-PostgreSQL engine. PostgreSQL nests block
    // comments, so `/* a /* b */ SELECT 1` really is unterminated there.
    // MySQL, SQLite, MSSQL, Oracle and Snowflake do NOT nest: for them the
    // comment ends at the first `*/` and the expected output is `SELECT 1`.
    // The tokenizer has no dialect input, so the query silently disappears.
    assert_eq!(clean_sql("/* a /* b */ SELECT 1"), "");
}

#[test]
fn clean_sql_does_not_treat_a_double_dash_inside_a_string_literal_as_a_comment() {
    assert_eq!(clean_sql("SELECT 'a -- b' FROM t"), "SELECT 'a -- b' FROM t");
}

#[test]
fn clean_sql_does_not_treat_a_double_dash_inside_a_dollar_quote_as_a_comment() {
    assert_eq!(clean_sql("SELECT $$a -- b$$ FROM t"), "SELECT $$a -- b$$ FROM t");
}

#[test]
fn clean_sql_does_not_treat_a_block_comment_open_inside_a_string_as_a_comment() {
    assert_eq!(clean_sql("SELECT '/*' FROM t"), "SELECT '/*' FROM t");
}

#[test]
fn clean_sql_does_not_let_an_apostrophe_inside_a_line_comment_open_a_string() {
    assert_eq!(clean_sql("-- it's fine\nSELECT 1"), "SELECT 1");
}

#[test]
fn clean_sql_does_not_treat_a_minus_minus_that_is_two_operators_as_a_comment() {
    // `1--2` is a comment in every SQL dialect (the lexer is greedy), so
    // this is correct, not a wart: it is pinned because a tokenizer rewrite
    // could plausibly get it wrong in the other direction.
    assert_eq!(clean_sql("SELECT 1--2"), "SELECT 1");
    assert_eq!(clean_sql("SELECT 1 - -2"), "SELECT 1 - -2");
}

#[test]
fn clean_sql_leaves_a_mysql_hash_comment_in_the_output() {
    // WRONG for MySQL/MariaDB, where `#` starts a line comment. The
    // tokenizer deliberately does not recognize `#` because it collides with
    // PostgreSQL's `#>`/`#>>` JSON operators -- which is exactly the
    // ambiguity dialect threading resolves. Expected under MySQL:
    // `SELECT 1\nFROM t`.
    assert_eq!(clean_sql("SELECT 1 # note\nFROM t"), "SELECT 1 # note\nFROM t");
}

#[test]
fn clean_sql_treats_a_double_dash_inside_a_mysql_backtick_identifier_as_a_comment() {
    // WRONG for MySQL/MariaDB: backticks quote an identifier, so `-- b` here
    // is part of the column name and the expected output is the input
    // unchanged. Because the tokenizer does not know backticks, it starts a
    // line comment mid-identifier and deletes the rest of the query.
    assert_eq!(clean_sql("SELECT `a -- b` FROM t"), "SELECT `a");
}

#[test]
fn clean_sql_treats_a_double_dash_inside_an_mssql_bracket_identifier_as_a_comment() {
    // WRONG for MSSQL: `[a -- b]` is a delimited identifier. Expected: input
    // unchanged.
    assert_eq!(clean_sql("SELECT [a -- b] FROM t"), "SELECT [a");
}

#[test]
fn clean_sql_handles_non_ascii_inside_comments_and_string_literals() {
    let sql = "SELECT name -- 名前を取得\nFROM t WHERE label = 'héllo wörld';";
    assert_eq!(clean_sql(sql), "SELECT name \nFROM t WHERE label = 'héllo wörld'");
}

// ---------------------------------------------------------------------------
// rewrite_pg_placeholders: $N numbering
// ---------------------------------------------------------------------------

#[test]
fn dollar_placeholder_is_rewritten_through_the_formatter() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT * FROM t WHERE id = $1", mark),
        "SELECT * FROM t WHERE id = [P1]"
    );
}

#[test]
fn multi_digit_placeholder_is_matched_greedily_as_one_number() {
    assert_eq!(rewrite_pg_placeholders("SELECT $10", mark), "SELECT [P10]");
}

#[test]
fn dollar_one_followed_by_a_separate_zero_token_is_still_read_as_ten() {
    // Not a defect -- `$1 0` needs the space, and `$10` is unambiguously
    // parameter 10 in PostgreSQL. Pinned because greedy-vs-single-digit is
    // the classic off-by-one in a rewrite.
    assert_eq!(rewrite_pg_placeholders("SELECT $1 0", mark), "SELECT [P1] 0");
    assert_eq!(rewrite_pg_placeholders("SELECT $10", mark), "SELECT [P10]");
}

#[test]
fn digits_stop_the_placeholder_at_the_first_non_digit() {
    assert_eq!(rewrite_pg_placeholders("SELECT $1x", mark), "SELECT [P1]x");
}

#[test]
fn a_repeated_placeholder_is_rewritten_at_every_occurrence() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT $1 FROM t WHERE a = $1", mark),
        "SELECT [P1] FROM t WHERE a = [P1]"
    );
}

#[test]
fn out_of_order_placeholders_keep_their_own_numbers() {
    assert_eq!(
        rewrite_pg_placeholders("WHERE a = $2 AND b = $1", mark),
        "WHERE a = [P2] AND b = [P1]"
    );
}

#[test]
fn gaps_in_the_placeholder_sequence_are_preserved_not_renumbered() {
    assert_eq!(
        rewrite_pg_placeholders("WHERE a = $1 AND b = $3", mark),
        "WHERE a = [P1] AND b = [P3]"
    );
}

#[test]
fn dollar_zero_is_rewritten_as_parameter_zero() {
    // WRONG: `$0` is not a legal PostgreSQL placeholder (parameters are
    // 1-based). It should be rejected or copied through, not silently
    // rewritten into a parameter reference the driver cannot bind.
    assert_eq!(rewrite_pg_placeholders("SELECT $0", mark), "SELECT [P0]");
}

#[test]
fn a_placeholder_number_too_large_for_u32_silently_becomes_parameter_zero() {
    // WRONG: `num_str.parse().unwrap_or(0)` turns any out-of-range number
    // into parameter 0, so `$4294967296` is emitted as a binding to
    // parameter 0. This should be an error, not a silent collision with
    // whatever `formatter(0)` produces.
    assert_eq!(rewrite_pg_placeholders("SELECT $4294967296", mark), "SELECT [P0]");
}

#[test]
fn a_leading_zero_placeholder_parses_as_its_numeric_value() {
    assert_eq!(rewrite_pg_placeholders("SELECT $01", mark), "SELECT [P1]");
}

#[test]
fn adjacent_placeholders_with_no_separator_are_both_rewritten() {
    // `$1$2` is not a dollar-quote open (a dollar-quote tag may not start
    // with a digit), so both are read as placeholders.
    assert_eq!(rewrite_pg_placeholders("SELECT $1$2", mark), "SELECT [P1][P2]");
}

#[test]
fn a_dollar_inside_an_identifier_is_rewritten_as_a_placeholder() {
    // WRONG: `$` is a legal identifier character after the first position in
    // PostgreSQL, Oracle and MySQL, so `amount$1` is a column name and the
    // expected output is the input unchanged. The char-scanner has no
    // preceding-token context and corrupts the identifier.
    assert_eq!(
        rewrite_pg_placeholders("SELECT amount$1 FROM t", mark),
        "SELECT amount[P1] FROM t"
    );
}

#[test]
fn a_lone_dollar_sign_is_copied_through_untouched() {
    assert_eq!(rewrite_pg_placeholders("SELECT $ , $1", mark), "SELECT $ , [P1]");
    assert_eq!(rewrite_pg_placeholders("SELECT $", mark), "SELECT $");
}

#[test]
fn sql_with_no_placeholders_is_returned_unchanged() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT id, name FROM users", mark),
        "SELECT id, name FROM users"
    );
}

#[test]
fn empty_sql_rewrites_to_empty_sql() {
    assert_eq!(rewrite_pg_placeholders("", mark), "");
}

// ---------------------------------------------------------------------------
// rewrite_pg_placeholders: string literals
// ---------------------------------------------------------------------------

#[test]
fn placeholder_inside_a_single_quoted_literal_is_not_rewritten() {
    assert_eq!(rewrite_pg_placeholders("SELECT '$1' , $2", mark), "SELECT '$1' , [P2]");
}

#[test]
fn doubled_single_quote_escape_does_not_end_the_literal_early() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT 'it''s $1 here' , $2", mark),
        "SELECT 'it''s $1 here' , [P2]"
    );
}

#[test]
fn question_mark_inside_a_single_quoted_literal_is_not_a_placeholder() {
    assert_eq!(rewrite_pg_placeholders("SELECT '?' , ?", mark), "SELECT '?' , [P1]");
}

#[test]
fn a_named_colon_parameter_is_never_rewritten_inside_or_outside_a_literal() {
    // The core parser rewrites Oracle `:name` to `?` before this function is
    // reached, so `:name` reaching here is copied through verbatim.
    assert_eq!(
        rewrite_pg_placeholders("SELECT ':name' , :name", mark),
        "SELECT ':name' , :name"
    );
}

#[test]
fn a_backslash_does_not_escape_the_closing_quote_of_a_standard_string() {
    // Correct for PostgreSQL with `standard_conforming_strings = on` (the
    // default since 9.1): `'a\'` is a complete literal whose last character
    // is a backslash, so the following `$1` really is a placeholder.
    // WRONG for MySQL/MariaDB, where `\'` escapes the quote by default and
    // the literal continues -- expected there: the whole tail is string
    // content and `$1` is left alone.
    assert_eq!(
        rewrite_pg_placeholders("SELECT 'a\\' , $1", mark),
        "SELECT 'a\\' , [P1]"
    );
}

#[test]
fn a_postgres_e_string_backslash_escape_swallows_the_rest_of_the_query() {
    // WRONG: `E'it\'s'` is a single PostgreSQL escape-string literal
    // containing `it's`, so the trailing `$1` is a placeholder and the
    // expected output ends `, [P1]`. The tokenizer does not know about the
    // `E` prefix, closes the literal at the escaped quote, then opens a new
    // unterminated literal at the real closing quote -- which swallows the
    // placeholder and everything after it.
    assert_eq!(
        rewrite_pg_placeholders("SELECT E'it\\'s' , $1", mark),
        "SELECT E'it\\'s' , $1"
    );
}

#[test]
fn an_unterminated_string_literal_swallows_every_later_placeholder() {
    assert_eq!(rewrite_pg_placeholders("SELECT 'abc , $1", mark), "SELECT 'abc , $1");
}

// ---------------------------------------------------------------------------
// rewrite_pg_placeholders: dollar-quoted strings
// ---------------------------------------------------------------------------

#[test]
fn dollar_quoted_body_is_not_placeholder_rewritten() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT $$ $1 $$ , $2", mark),
        "SELECT $$ $1 $$ , [P2]"
    );
}

#[test]
fn tagged_dollar_quoted_body_is_not_placeholder_rewritten() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT $tag$ $1 $tag$ , $2", mark),
        "SELECT $tag$ $1 $tag$ , [P2]"
    );
}

#[test]
fn a_tag_may_contain_digits_after_its_first_character() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT $a1$ $1 $a1$ , $2", mark),
        "SELECT $a1$ $1 $a1$ , [P2]"
    );
}

#[test]
fn a_tag_starting_with_a_digit_is_not_a_dollar_quote_open() {
    // `$1$` cannot open a dollar quote (tags follow identifier rules), so
    // `$1` is a placeholder and the trailing `$x` text stays literal.
    assert_eq!(rewrite_pg_placeholders("SELECT $1$ x", mark), "SELECT [P1]$ x");
}

#[test]
fn an_inner_tag_does_not_close_an_outer_dollar_quote() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT $a$ $b$ $1 $b$ $a$ , $2", mark),
        "SELECT $a$ $b$ $1 $b$ $a$ , [P2]"
    );
}

#[test]
fn an_empty_dollar_quote_consumes_exactly_four_dollar_signs() {
    assert_eq!(rewrite_pg_placeholders("SELECT $$$$ , $1", mark), "SELECT $$$$ , [P1]");
}

#[test]
fn a_mismatched_closing_tag_swallows_every_later_placeholder() {
    // WRONG-adjacent: PostgreSQL would reject this SQL outright, but scythe
    // silently emits a statement whose remaining placeholders were never
    // rewritten -- a runtime bind failure instead of a compile-time error.
    // Expected: an error, not silent pass-through.
    assert_eq!(
        rewrite_pg_placeholders("SELECT $a$ x $b$ , $1", mark),
        "SELECT $a$ x $b$ , $1"
    );
}

#[test]
fn an_unterminated_dollar_quote_swallows_every_later_placeholder() {
    assert_eq!(rewrite_pg_placeholders("SELECT $$ x , $1", mark), "SELECT $$ x , $1");
}

#[test]
fn a_question_mark_inside_a_dollar_quoted_body_is_not_a_placeholder() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT $$ ? $$ , ?", mark),
        "SELECT $$ ? $$ , [P1]"
    );
}

// ---------------------------------------------------------------------------
// rewrite_pg_placeholders: comments
// ---------------------------------------------------------------------------

#[test]
fn placeholder_inside_a_line_comment_is_not_rewritten() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT 1 -- $1\n, $2", mark),
        "SELECT 1 -- $1\n, [P2]"
    );
}

#[test]
fn placeholder_inside_a_block_comment_is_not_rewritten() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT /* $1 */ $2", mark),
        "SELECT /* $1 */ [P2]"
    );
}

#[test]
fn placeholder_inside_a_nested_block_comment_is_not_rewritten() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT /* a /* $1 */ b */ $2", mark),
        "SELECT /* a /* $1 */ b */ [P2]"
    );
}

#[test]
fn an_apostrophe_inside_a_line_comment_does_not_open_a_string_literal() {
    assert_eq!(
        rewrite_pg_placeholders("-- it's\nSELECT $1", mark),
        "-- it's\nSELECT [P1]"
    );
}

#[test]
fn a_question_mark_inside_a_comment_does_not_advance_the_positional_counter() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT ? -- ?\n, ?", mark),
        "SELECT [P1] -- ?\n, [P2]"
    );
}

// ---------------------------------------------------------------------------
// rewrite_pg_placeholders: quoted identifiers
// ---------------------------------------------------------------------------

#[test]
fn placeholder_inside_a_double_quoted_identifier_is_not_rewritten() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT \"col$1\" FROM t WHERE a = $2", mark),
        "SELECT \"col$1\" FROM t WHERE a = [P2]"
    );
}

#[test]
fn a_doubled_double_quote_does_not_end_a_quoted_identifier_early() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT \"a\"\"b $1\" , $2", mark),
        "SELECT \"a\"\"b $1\" , [P2]"
    );
}

#[test]
fn question_mark_inside_a_double_quoted_identifier_is_not_a_placeholder() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT \"a?b\" FROM t WHERE c = ?", mark),
        "SELECT \"a?b\" FROM t WHERE c = [P1]"
    );
}

#[test]
fn question_mark_inside_a_mysql_backtick_identifier_is_rewritten_as_a_placeholder() {
    // WRONG for MySQL/MariaDB: backticks quote an identifier, so `a?b` is a
    // column name and only the trailing `?` is a placeholder. Expected:
    // "SELECT `a?b` FROM t WHERE c = [P1]". The tokenizer does not know
    // backticks, corrupts the identifier, and shifts every later parameter
    // number by one.
    assert_eq!(
        rewrite_pg_placeholders("SELECT `a?b` FROM t WHERE c = ?", mark),
        "SELECT `a[P1]b` FROM t WHERE c = [P2]"
    );
}

#[test]
fn question_mark_inside_an_mssql_bracket_identifier_is_rewritten_as_a_placeholder() {
    // WRONG for MSSQL: `[a?b]` is a delimited identifier. Expected:
    // "SELECT [a?b] FROM t WHERE c = [P1]".
    assert_eq!(
        rewrite_pg_placeholders("SELECT [a?b] FROM t WHERE c = ?", mark),
        "SELECT [a[P1]b] FROM t WHERE c = [P2]"
    );
}

// ---------------------------------------------------------------------------
// rewrite_pg_placeholders: bare `?` and the dialect heuristic
// ---------------------------------------------------------------------------

#[test]
fn bare_question_marks_are_numbered_sequentially_when_no_dollar_placeholder_exists() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT * FROM t WHERE a = ? AND b = ?", mark),
        "SELECT * FROM t WHERE a = [P1] AND b = [P2]"
    );
}

#[test]
fn a_bare_question_mark_is_left_alone_when_the_query_also_uses_dollar_placeholders() {
    assert_eq!(
        rewrite_pg_placeholders("WHERE a = $1 AND data ? 'k'", mark),
        "WHERE a = [P1] AND data ? 'k'"
    );
}

#[test]
fn the_dollar_heuristic_is_global_so_a_late_dollar_placeholder_protects_an_earlier_question_mark() {
    assert_eq!(
        rewrite_pg_placeholders("WHERE data ? 'k' AND a = $1", mark),
        "WHERE data ? 'k' AND a = [P1]"
    );
}

#[test]
fn a_jsonb_key_existence_operator_is_rewritten_when_the_query_has_no_dollar_placeholder() {
    // WRONG: `data ? 'active'` is PostgreSQL's JSONB key-existence operator,
    // not a placeholder, and the expected output is the input unchanged.
    // With zero `$N` in the query the heuristic has nothing to anchor on and
    // guesses "this is a `?`-style dialect". This is the single case that
    // most needs the dialect threaded through, and it is already documented
    // as a residual limitation in the source.
    assert_eq!(
        rewrite_pg_placeholders("SELECT * FROM docs WHERE data ? 'active'", mark),
        "SELECT * FROM docs WHERE data [P1] 'active'"
    );
}

#[test]
fn multi_char_jsonb_operators_survive_even_with_no_dollar_placeholder_to_anchor_on() {
    assert_eq!(
        rewrite_pg_placeholders(
            "SELECT * FROM docs WHERE tags ?| ARRAY['a'] AND meta ?& ARRAY['b']",
            mark
        ),
        "SELECT * FROM docs WHERE tags ?| ARRAY['a'] AND meta ?& ARRAY['b']"
    );
}

#[test]
fn geometry_operators_survive_and_the_longest_match_wins() {
    assert_eq!(
        rewrite_pg_placeholders("WHERE a ?-| b AND c ?- d AND e ?|| f", mark),
        "WHERE a ?-| b AND c ?- d AND e ?|| f"
    );
}

#[test]
fn the_jsonpath_exists_operator_survives() {
    assert_eq!(
        rewrite_pg_placeholders("WHERE doc @? '$.a'", mark),
        "WHERE doc @? '$.a'"
    );
}

#[test]
fn a_skipped_operator_does_not_advance_the_positional_counter() {
    assert_eq!(
        rewrite_pg_placeholders("WHERE tags ?| ARRAY['a'] AND b = ?", mark),
        "WHERE tags ?| ARRAY['a'] AND b = [P1]"
    );
}

#[test]
fn a_question_mark_placeholder_directly_followed_by_a_concat_operator_is_not_rewritten() {
    // WRONG for SQLite/MySQL/Oracle: `?||'x'` is a positional placeholder
    // concatenated with a literal, and the expected output is
    // "SELECT [P1]||'x'". Because `?||` is on the unconditional
    // pass-through list it is copied verbatim and the parameter is never
    // bound -- the driver then sees fewer placeholders than arguments.
    assert_eq!(rewrite_pg_placeholders("SELECT ?||'x'", mark), "SELECT ?||'x'");
}

#[test]
fn a_question_mark_placeholder_directly_followed_by_a_minus_is_not_rewritten() {
    // WRONG for SQLite/MySQL/Oracle: `?-1` is "placeholder minus one" and
    // should become "SELECT [P1]-1". `?-` is on the unconditional
    // pass-through list because it is a PostgreSQL geometry operator.
    assert_eq!(rewrite_pg_placeholders("SELECT ?-1", mark), "SELECT ?-1");
}

#[test]
fn a_question_mark_followed_by_a_digit_is_never_rewritten() {
    // WRONG for SQLite, where `?1`/`?2` are explicitly numbered positional
    // parameters -- expected "SELECT [P1], [P2]". They are skipped, and
    // they also do not advance the counter, so a mixed `?1, ?` query
    // misnumbers the bare `?`.
    assert_eq!(rewrite_pg_placeholders("SELECT ?1, ?2", mark), "SELECT ?1, ?2");
    assert_eq!(rewrite_pg_placeholders("SELECT ?1, ?", mark), "SELECT ?1, [P1]");
}

#[test]
fn the_positional_counter_runs_across_string_and_comment_spans_in_order() {
    assert_eq!(
        rewrite_pg_placeholders("SELECT ?, '?' , ? /* ? */, ?", mark),
        "SELECT [P1], '?' , [P2] /* ? */, [P3]"
    );
}

#[test]
fn a_question_mark_at_the_very_end_of_the_input_is_rewritten() {
    assert_eq!(rewrite_pg_placeholders("SELECT ?", mark), "SELECT [P1]");
}
