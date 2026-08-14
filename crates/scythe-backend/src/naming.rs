use std::borrow::Cow;

use serde::Deserialize;

/// Naming conventions for generated code.
#[derive(Debug, Clone, Deserialize)]
pub struct NamingConfig {
    pub struct_case: String,
    pub fn_case: String,
    pub enum_variant_case: String,
    pub row_suffix: String,
    /// Case convention for struct/row and function field names (columns and
    /// params).
    ///
    /// Deliberately `#[serde(skip)]`: this field cannot be set from manifest
    /// TOML at all -- a `field_case` key under `[naming]` in a manifest is
    /// silently ignored by serde as an unknown field, exactly as before this
    /// field existed. A prior version of this option was a plain
    /// deserialized field, declared in every manifest and read by
    /// nothing (see naming.rs history), so the dead knob was invisible.
    /// `serde(skip)` makes that trap structurally impossible to reintroduce:
    /// the only writer is a backend's `apply_options`, so a value can never
    /// reach here without something in Rust actually reading it back out.
    #[serde(skip, default = "default_field_case")]
    pub field_case: String,
    /// Target-language keywords that are not valid (or not safe) bare
    /// identifiers, consulted by [`field_name`].
    ///
    /// Driven entirely by the manifest -- there is deliberately no
    /// hardcoded, cross-language table here. A hardcoded table that drifts
    /// out of sync with what a manifest actually declares is its own bug
    /// (#198); the fix is that this list has exactly one source; the
    /// manifest's `[naming] reserved = [...]` array. Defaults to empty so a
    /// manifest that has not opted in is unaffected.
    #[serde(default)]
    pub reserved: Vec<String>,
    /// Target-language words that are illegal specifically where a generated
    /// name lands in a *binding* position -- a function parameter, a `const`,
    /// a destructuring target -- while remaining legal as a property key.
    ///
    /// Only the TypeScript manifests declare this, because TypeScript is the
    /// one target here where the two positions have different rules:
    /// `interface R { class: string }` and `row.class` are both legal, but
    /// `function q(class: string)` is `TS1390`. Listing `class` in
    /// [`Self::reserved`] instead would mangle the *column* too, and a
    /// generated TypeScript row type is cast straight onto the driver's rows
    /// (`client.query<CreateOrderRow>(...)`), so a key that no longer matches
    /// the SQL column name describes an object the driver never returns --
    /// trading a compile error for a silent wrong answer.
    ///
    /// For every other target a keyword is illegal in both positions and so
    /// belongs in [`Self::reserved`], which [`field_name`] already applies to
    /// params as well. Consulted only by [`param_name`]; defaults to empty,
    /// so a manifest that has not opted in is unaffected.
    #[serde(default)]
    pub reserved_bindings: Vec<String>,
    /// Whether [`field_name`] should repair identifier *shape* -- replace the
    /// characters an identifier cannot hold, and prefix a leading digit.
    ///
    /// Opt-in, and declared by every manifest whose target has no quoted form
    /// for a field name, which is all of them but TypeScript. A column named
    /// `my col` otherwise reaches `pub my col: String`, `my col: str`,
    /// `My col string` and `String my col` -- none of which parse. Those
    /// targets read the column back positionally or by its raw SQL name
    /// (`rs.getString("my col")`), so the generated field name is theirs to
    /// choose and mangling costs nothing.
    ///
    /// The TypeScript manifests leave it off because they have a better
    /// answer -- `"my col": string` and `row["my col"]` are both legal, and
    /// #215 already routes every such position through them -- and because
    /// mangling would be actively wrong there: a TypeScript row type is cast
    /// onto the driver's rows, so a renamed key describes an object that
    /// never arrives.
    ///
    /// Opt-in rather than opt-out so the default failure is a compile error
    /// in a target that has not been considered, never a silent rename.
    #[serde(default)]
    pub sanitize_field_names: bool,
}

fn default_field_case() -> String {
    "snake_case".to_string()
}

/// Convert a string to PascalCase.
///
/// Handles snake_case input ("user_status" -> "UserStatus")
/// and already-PascalCase input ("UserStatus" -> "UserStatus").
///
/// Normalizes through [`to_snake_case`] first: it is the only converter
/// here with real consecutive-capital handling, so it is what turns
/// "CreateAPIKey" into word boundaries this can rebuild from. Without it,
/// mixed-case input with no underscore was returned unchanged
/// ("CreateAPIKey" -> "CreateAPIKey"), which left `struct_case =
/// "PascalCase"` and `fn_case = "camelCase"` disagreeing: the row type
/// stayed `CreateAPIKeyRow` while the function became `createApiKey`. Both
/// now derive from the same snake_case stem, so a query's function and its
/// row type always spell the name the same way.
pub fn to_pascal_case(s: &str) -> Cow<'_, str> {
    let snake = to_snake_case(s);
    let mut result = String::with_capacity(snake.len());
    for part in snake.split('_') {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            result.extend(first.to_uppercase());
            // ~keep `snake` is already lowercased, so the tail needs no further
            // case folding.
            result.push_str(chars.as_str());
        }
    }
    // ~keep Reachable only when every `_`-delimited part was itself empty --
    // `snake` is "_", "__", or a symbols-only value that
    // `sanitize_for_identifier` turned into a run of underscores (an enum
    // label of just "!" or "."). Returning "" there emits `pub enum {}` --
    // a type with no name -- a syntax error in every target language.
    // `to_camel_case` already falls back this way for the identical shape;
    // this makes the two agree instead of `to_camel_case` papering over a gap
    // `to_pascal_case` leaves open.
    if result.is_empty() && !snake.is_empty() {
        return Cow::Owned(snake.into_owned());
    }
    // Keep the borrow when the input was already PascalCase; callers rely on
    // it to avoid an allocation per column and per query name.
    if result == s {
        return Cow::Borrowed(s);
    }
    Cow::Owned(result)
}

/// Convert a string to snake_case.
///
/// Handles PascalCase input ("UserStatus" -> "user_status")
/// and already-snake_case input ("user_status" -> "user_status").
/// Correctly handles consecutive uppercase letters:
/// "HTTPClient" -> "http_client", "UserID" -> "user_id".
pub fn to_snake_case(s: &str) -> Cow<'_, str> {
    if s.contains('_') {
        let lower = s.to_lowercase();
        if lower == s {
            return Cow::Borrowed(s);
        }
        return Cow::Owned(lower);
    }

    if s.chars().all(|c| !c.is_uppercase()) {
        return Cow::Borrowed(s);
    }

    let mut result = String::with_capacity(s.len() + 4);
    let mut prev_char: Option<char> = None;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c.is_uppercase() {
            if let Some(prev) = prev_char {
                let prev_upper = prev.is_uppercase();
                let next_lower = chars.peek().is_some_and(|ch| ch.is_lowercase());
                if !prev_upper || next_lower {
                    result.push('_');
                }
            }
            result.extend(c.to_lowercase());
        } else {
            result.push(c);
        }
        prev_char = Some(c);
    }
    Cow::Owned(result)
}

/// Convert a string to camelCase.
///
/// Handles snake_case input ("user_status" -> "userStatus")
/// and PascalCase input ("UserStatus" -> "userStatus").
///
/// This is [`to_pascal_case`] with the first character lowercased, so it
/// inherits that function's normalization through [`to_snake_case`] and is
/// its exact inverse: "HTTPSUrl" -> "httpsUrl", not the "hTTPSUrl" that
/// lowercasing an unnormalized "HTTPSUrl" would give.
pub fn to_camel_case(s: &str) -> Cow<'_, str> {
    let pascal = to_pascal_case(s);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(c) => {
            let mut result = String::with_capacity(pascal.len());
            result.extend(c.to_lowercase());
            result.push_str(chars.as_str());
            Cow::Owned(result)
        }
        // ~keep Reachable only when `s` itself is empty: `to_pascal_case` now
        // falls back to its sanitized input rather than "" for a
        // symbols-only value (e.g. "__"), so this arm no longer has to catch
        // that case too.
        None => Cow::Borrowed(s),
    }
}

/// Convert a string to SCREAMING_SNAKE_CASE.
///
/// Handles any input by converting to snake_case first, then uppercasing.
/// "active" -> "ACTIVE", "user_status" -> "USER_STATUS", "PascalCase" -> "PASCAL_CASE"
pub fn to_screaming_snake_case(s: &str) -> Cow<'_, str> {
    let snake = to_snake_case(s);
    Cow::Owned(snake.to_uppercase())
}

/// Apply a named case convention to a string.
pub fn apply_case<'a>(s: &'a str, case: &str) -> Cow<'a, str> {
    match case {
        "PascalCase" => to_pascal_case(s),
        "snake_case" => to_snake_case(s),
        "camelCase" => to_camel_case(s),
        "SCREAMING_SNAKE_CASE" => to_screaming_snake_case(s),
        _ => Cow::Borrowed(s),
    }
}

/// Generate the row struct name for a query.
///
/// E.g., query "ListUsers" with suffix "Row" and PascalCase -> "ListUsersRow"
pub fn row_struct_name(query_name: &str, naming: &NamingConfig) -> String {
    let base = apply_case(query_name, &naming.struct_case);
    format!("{}{}", base, naming.row_suffix)
}

/// Generate the function name for a query.
///
/// E.g., query "GetUser" with snake_case -> "get_user"
pub fn fn_name(query_name: &str, naming: &NamingConfig) -> String {
    apply_case(query_name, &naming.fn_case).into_owned()
}

/// Generate the type name for an enum from its SQL name.
///
/// E.g., sql name "user_status" with PascalCase -> "UserStatus".
///
/// The SQL name reaching here is not guaranteed to already be an identifier:
/// a schema-qualified enum (`CREATE TYPE app.status AS ENUM (...)`) carries
/// its qualifying `.` straight into `EnumInfo::sql_name`, and `apply_case`
/// alone does not remove it -- `to_pascal_case("app.status")` was
/// `"App.status"`, a `.` inside `pub enum App.status` in every backend that
/// shares this function (#136). Routed through the same
/// [`sanitize_for_identifier`] [`enum_variant_name`] already uses, so the
/// `.` becomes `_` before casing runs, exactly like a variant label's `-`/`.`
/// already do.
pub fn enum_type_name(sql_name: &str, naming: &NamingConfig) -> String {
    let sanitized = sanitize_for_identifier(sql_name);
    apply_case(&sanitized, &naming.struct_case).into_owned()
}

/// Generate a field name (column or param) from its SQL name.
///
/// E.g., sql name "user_id" with camelCase -> "userId". Defaults to
/// `snake_case` -- see [`NamingConfig::field_case`].
///
/// When the case-converted name exactly matches one of the manifest's
/// [`NamingConfig::reserved`] target-language keywords (e.g. SQL column
/// `type` emitting as the Rust keyword `type`, or `class` as the Python
/// keyword `class`), a trailing underscore is appended -- a suffix every
/// target language in this crate accepts as an ordinary identifier
/// character, so one mangling strategy works everywhere without a
/// per-language special case. A manifest with an empty `reserved` list (the
/// default) never mangles anything.
///
/// Identifier *shape* is repaired only when the manifest sets
/// [`NamingConfig::sanitize_field_names`] -- see that field for why it is
/// opt-in rather than universal.
pub fn field_name<'a>(sql_name: &'a str, naming: &NamingConfig) -> Cow<'a, str> {
    if naming.sanitize_field_names {
        return Cow::Owned(identifier_name(sql_name, naming));
    }
    let cased = apply_case(sql_name, &naming.field_case);
    if naming.reserved.iter().any(|kw| kw == cased.as_ref()) {
        Cow::Owned(format!("{cased}_"))
    } else {
        cased
    }
}

/// Case convention, identifier shape and reserved-word suffix, each applied
/// in the only order that works.
///
/// The characters are replaced *first*, so `_` is already the word separator
/// the case converters split on: `with-dash` becomes `withDash` under
/// camelCase rather than staying `with-dash`. The leading-digit guard is
/// applied to that same pre-case spelling, so the case conversion carries it
/// through: `2fa` becomes `col_2fa`, and therefore `Col2fa` wherever a
/// backend PascalCases the field. The reserved-word check runs last, on the
/// final cased name, because that is the spelling that has to avoid the
/// keyword.
///
/// The guard is a word and not a bare `_` because a leading underscore does
/// not survive a case conversion: `to_pascal_case` splits on `_` and an
/// empty leading part contributes nothing, so `_2fa` came back out as `2fa`
/// in go-pgx and the csharp family -- and teaching the converter to keep it
/// would change `_user_status` from `UserStatus` to `_UserStatus`
/// everywhere, which two tests here pin deliberately.
fn identifier_name(sql_name: &str, naming: &NamingConfig) -> String {
    let mut shaped = replace_non_identifier_chars(sql_name);
    if shaped.starts_with(|c: char| c.is_ascii_digit()) {
        shaped.insert_str(0, "col_");
    }
    let cased = apply_case(&shaped, &naming.field_case);
    if naming.reserved.iter().any(|kw| kw == cased.as_ref()) {
        format!("{cased}_")
    } else {
        cased.into_owned()
    }
}

/// Generate the name a query parameter binds to in generated code.
///
/// [`field_name`] plus the manifest's [`NamingConfig::reserved_bindings`].
/// Params are the only generated names that land in a binding position,
/// where a target-language keyword cannot be quoted out of trouble the way
/// a property key can.
///
/// Also the one place identifier *shape* is repaired: a param named after a
/// column called `my col`, `with-dash` or `2fa` binds as `my_col`,
/// `with_dash` and `col_2fa`. Quoting is what saves the other positions -- a
/// property key can be `"my col"`, a read can be `row["my col"]` -- and a
/// binding is the position with no quoted form, in any of the ten target
/// languages.
///
/// Mangling is safe here in a way it is not for a column: a param name is
/// generated code's own vocabulary -- bound into the SQL positionally, read
/// back by nobody -- whereas a column's field name is a contract with
/// whatever the driver hands back. Any collision the extra suffix or the
/// character replacement introduces is caught by
/// `resolve::check_field_name_collisions`, which runs over the names this
/// returns, so `my col` colliding with a real `my_col` is an error and not a
/// silent overwrite.
///
/// Unconditional, unlike [`field_name`]'s
/// [`NamingConfig::sanitize_field_names`] opt-in: quoting is what lets a
/// manifest decline shape repair for a *field*, and a binding has no quoted
/// form in any target language, so there is nothing to opt out of.
pub fn param_name(sql_name: &str, naming: &NamingConfig) -> String {
    let mut name = identifier_name(sql_name, naming);
    if naming.reserved_bindings.contains(&name) {
        name.push('_');
    }
    name
}

/// Replace every character that cannot appear in an identifier with `_`.
///
/// Shared by [`param_name`] and [`sanitize_for_identifier`] so the rule for
/// what counts as an identifier character has one derivation. What each does
/// about a *leading digit* is deliberately not shared: an enum variant is
/// PascalCase and takes a `V` prefix, a param binding takes `_`.
fn replace_non_identifier_chars(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            result.push(ch);
        } else {
            result.push('_');
        }
    }
    result
}

/// Sanitize a string to be a valid Rust identifier fragment.
///
/// Replaces hyphens, dots, and other non-alphanumeric/non-underscore characters
/// with underscores, and prefixes with `V` if the result starts with a digit.
fn sanitize_for_identifier(s: &str) -> String {
    let mut result = replace_non_identifier_chars(s);
    if result.starts_with(|c: char| c.is_ascii_digit()) {
        result.insert(0, 'V');
    }
    result
}

/// Generate an enum variant name from its SQL value.
///
/// E.g., sql value "active" with PascalCase -> "Active"
/// Handles special characters: "gpt-3.5-turbo" -> "Gpt3_5Turbo", "PG-13" -> "Pg13"
pub fn enum_variant_name(sql_value: &str, naming: &NamingConfig) -> String {
    let sanitized = sanitize_for_identifier(sql_value);
    apply_case(&sanitized, &naming.enum_variant_case).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> NamingConfig {
        NamingConfig {
            struct_case: "PascalCase".to_string(),
            fn_case: "snake_case".to_string(),
            enum_variant_case: "PascalCase".to_string(),
            row_suffix: "Row".to_string(),
            field_case: "snake_case".to_string(),
            reserved: Vec::new(),
            reserved_bindings: Vec::new(),
            sanitize_field_names: false,
        }
    }

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(&*to_pascal_case("user_status"), "UserStatus");
        assert_eq!(&*to_pascal_case("order_items"), "OrderItems");
        assert_eq!(&*to_pascal_case("UserStatus"), "UserStatus");
        assert_eq!(&*to_pascal_case("active"), "Active");
    }

    #[test]
    fn test_to_pascal_case_borrows_when_unchanged() {
        assert!(matches!(to_pascal_case("UserStatus"), Cow::Borrowed(_)));
    }

    /// This must fail before the fix: `to_pascal_case` returned mixed-case
    /// input with no underscore unchanged, so "CreateAPIKey" stayed
    /// "CreateAPIKey" while `to_camel_case` (which does normalize) produced
    /// "createApiKey" — the same query's row type and function spelled its
    /// name differently. See [`test_fn_name_and_row_struct_name_agree`].
    #[test]
    fn test_to_pascal_case_normalizes_consecutive_capitals() {
        assert_eq!(&*to_pascal_case("CreateAPIKey"), "CreateApiKey");
        assert_eq!(&*to_pascal_case("RetrieveUserAccountByID"), "RetrieveUserAccountById");
        assert_eq!(&*to_pascal_case("HTTPSUrl"), "HttpsUrl");
        assert_eq!(&*to_pascal_case("ABCDef"), "AbcDef");
    }

    /// The all-uppercase and degenerate inputs `to_pascal_case` used to
    /// special-case with its own branches, now that everything routes
    /// through `to_snake_case`.
    #[test]
    fn test_to_pascal_case_degenerate() {
        assert_eq!(&*to_pascal_case(""), "");
        assert_eq!(&*to_pascal_case("a"), "A");
        assert_eq!(&*to_pascal_case("ID"), "Id");
        assert_eq!(&*to_pascal_case("USER_ID"), "UserId");
        assert_eq!(&*to_pascal_case("PG_13"), "Pg13");
    }

    #[test]
    fn test_to_snake_case() {
        assert_eq!(&*to_snake_case("UserStatus"), "user_status");
        assert_eq!(&*to_snake_case("user_status"), "user_status");
        assert_eq!(&*to_snake_case("GetUser"), "get_user");
        assert_eq!(&*to_snake_case("ListUsers"), "list_users");
    }

    #[test]
    fn test_to_snake_case_borrows_when_unchanged() {
        assert!(matches!(to_snake_case("user_status"), Cow::Borrowed(_)));
    }

    #[test]
    fn test_to_camel_case() {
        assert_eq!(&*to_camel_case("user_status"), "userStatus");
        assert_eq!(&*to_camel_case("UserStatus"), "userStatus");
        assert_eq!(&*to_camel_case("get_user"), "getUser");
    }

    /// This must fail before the fix: `to_camel_case` used to pascal-case
    /// directly, and `to_pascal_case` borrows mixed-case input with no
    /// underscore unchanged, so `to_camel_case("HTTPSUrl")` produced the
    /// broken "hTTPSUrl" (only the first letter lowercased) instead of
    /// "httpsUrl".
    #[test]
    fn test_to_camel_case_consecutive_capitals() {
        assert_eq!(&*to_camel_case("HTTPSUrl"), "httpsUrl");
        assert_eq!(&*to_camel_case("HTTPClient"), "httpClient");
        assert_eq!(&*to_camel_case("XMLParser"), "xmlParser");
        assert_eq!(&*to_camel_case("UserID"), "userId");
        assert_eq!(&*to_camel_case("getHTTPSUrl"), "getHttpsUrl");
        assert_eq!(&*to_camel_case("ABCDef"), "abcDef");
    }

    /// Four spellings of the same identifier must all collapse to the same
    /// camelCase name — the property that makes `field_case = "camelCase"`
    /// collision detection meaningful in `resolve.rs`.
    #[test]
    fn test_to_camel_case_collision_corpus_agrees() {
        for input in ["user_id", "USER_ID", "UserId", "userId"] {
            assert_eq!(&*to_camel_case(input), "userId", "input: {input}");
        }
    }

    #[test]
    fn test_to_camel_case_underscore_edges() {
        assert_eq!(&*to_camel_case("_id"), "id");
        assert_eq!(&*to_camel_case("id_"), "id");
        assert_eq!(&*to_camel_case("user__id"), "userId");
        // This must fail before the fix: an input that holds no word
        // characters at all used to camel-case to the empty string, which
        // reaches codegen as a field declaration with no name (`: number,`)
        // — a syntax error in the generated file. Preserving the input keeps
        // whatever identifier the raw SQL name already was.
        assert_eq!(&*to_camel_case("__"), "__");
    }

    #[test]
    fn test_to_camel_case_degenerate() {
        assert_eq!(&*to_camel_case(""), "");
        assert_eq!(&*to_camel_case("a"), "a");
        assert_eq!(&*to_camel_case("A"), "a");
        assert_eq!(&*to_camel_case("ID"), "id");
    }

    /// The corpus exercised by the other `to_camel_case` tests, reused here
    /// so the idempotence and inverse properties below cover the same inputs
    /// rather than a hand-picked subset.
    fn camel_case_corpus() -> &'static [&'static str] {
        &[
            "HTTPSUrl",
            "HTTPClient",
            "XMLParser",
            "UserID",
            "getHTTPSUrl",
            "ABCDef",
            "user_id",
            "USER_ID",
            "UserId",
            "userId",
            "_id",
            "id_",
            "user__id",
            "__",
            "",
            "a",
            "A",
            "ID",
            "ünique_id",
            "col_1",
            "1st_place",
        ]
    }

    #[test]
    fn test_to_camel_case_is_idempotent_over_corpus() {
        for input in camel_case_corpus() {
            let once = to_camel_case(input).into_owned();
            let twice = to_camel_case(&once).into_owned();
            assert_eq!(twice, once, "input: {input}");
        }
    }

    /// `to_camel_case` is meant to be the inverse of `to_snake_case`: running
    /// a name through `to_snake_case` first (as a manifest's `field_case`
    /// switch from snake_case to camelCase would) must not change what
    /// `to_camel_case` produces for it.
    #[test]
    fn test_to_camel_case_is_inverse_of_to_snake_case_over_corpus() {
        for input in camel_case_corpus() {
            let via_snake = to_camel_case(&to_snake_case(input)).into_owned();
            let direct = to_camel_case(input).into_owned();
            assert_eq!(via_snake, direct, "input: {input}");
        }
    }

    /// Guards the multi-char-aware `to_uppercase`/`to_lowercase` path: a
    /// naive byte-wise ASCII uppercase/lowercase would corrupt "ü".
    #[test]
    fn test_to_camel_case_non_ascii() {
        assert_eq!(&*to_camel_case("ünique_id"), "üniqueId");
    }

    #[test]
    fn test_to_camel_case_digit_edges() {
        assert_eq!(&*to_camel_case("col_1"), "col1");
        // FIXME: a leading digit is not a valid JS/Java identifier. This is
        // pre-existing and not fixed here: snake_case's "1st_place" is
        // equally invalid, so camelCase is not introducing a new problem —
        // just not solving an old one.
        assert_eq!(&*to_camel_case("1st_place"), "1stPlace");
    }

    #[test]
    fn test_fn_name() {
        let config = test_config();
        assert_eq!(fn_name("GetUser", &config), "get_user");
        assert_eq!(fn_name("ListUsers", &config), "list_users");
    }

    #[test]
    fn test_row_struct_name() {
        let config = test_config();
        assert_eq!(row_struct_name("GetUser", &config), "GetUserRow");
        assert_eq!(row_struct_name("ListUsers", &config), "ListUsersRow");
    }

    /// This must fail before the fix: `fn_case = "camelCase"` (set by 53 of
    /// the 102 manifests) renamed "CreateAPIKey" to "createApiKey" while
    /// `struct_case = "PascalCase"` left the row type "CreateAPIKeyRow", so
    /// a generated function and the row type it returns disagreed about the
    /// stem of the very same query name. Both derive from the same
    /// snake_case normalization now, so the only difference between them is
    /// the leading character's case and the row suffix.
    #[test]
    fn test_fn_name_and_row_struct_name_agree() {
        let mut config = test_config();
        config.fn_case = "camelCase".to_string();

        for query_name in [
            "CreateAPIKey",
            "RetrieveUserAccountByID",
            "RetrieveApplicationInternalAPIKeyID",
            "GetUser",
        ] {
            let function = fn_name(query_name, &config);
            let row_type = row_struct_name(query_name, &config);
            let mut chars = function.chars();
            let capitalized: String = chars
                .next()
                .map(|c| c.to_uppercase().to_string() + chars.as_str())
                .unwrap_or_default();
            let expected_row_type = format!("{}{}", capitalized, config.row_suffix);
            assert_eq!(row_type, expected_row_type, "query name: {query_name}");
        }

        assert_eq!(fn_name("CreateAPIKey", &config), "createApiKey");
        assert_eq!(row_struct_name("CreateAPIKey", &config), "CreateApiKeyRow");
    }

    #[test]
    fn test_enum_type_name() {
        let config = test_config();
        assert_eq!(enum_type_name("user_status", &config), "UserStatus");
    }

    #[test]
    fn test_field_name_defaults_to_snake_case() {
        let config = test_config();
        assert_eq!(&*field_name("UserId", &config), "user_id");
    }

    #[test]
    fn test_field_name_honors_camel_case() {
        let mut config = test_config();
        config.field_case = "camelCase".to_string();
        assert_eq!(&*field_name("user_id", &config), "userId");
    }

    /// Regression coverage for #180/#151: a column whose SQL name is a
    /// target-language keyword must not be emitted verbatim. This is the
    /// exact shape those issues reported -- `SELECT type, class FROM t`
    /// producing `pub type: String` / `String class` that fails to parse.
    #[test]
    fn test_field_name_mangles_a_reserved_word_with_a_trailing_underscore() {
        let mut config = test_config();
        config.reserved = vec!["type".to_string(), "class".to_string()];
        assert_eq!(&*field_name("type", &config), "type_");
        assert_eq!(&*field_name("class", &config), "class_");
    }

    /// A manifest that never declares `reserved` (the default, empty list)
    /// must not mangle anything -- this is the behavior every manifest had
    /// before #180/#151, and it must stay the default for a manifest that
    /// has not opted in.
    #[test]
    fn test_field_name_does_not_mangle_when_reserved_list_is_empty() {
        let config = test_config();
        assert!(config.reserved.is_empty());
        assert_eq!(&*field_name("type", &config), "type");
    }

    /// A word that is not in the manifest's reserved list -- however
    /// keyword-like it looks -- must pass through unmangled. The reserved
    /// list is the manifest's own vocabulary, not a guess.
    #[test]
    fn test_field_name_leaves_non_reserved_words_alone() {
        let mut config = test_config();
        config.reserved = vec!["type".to_string()];
        assert_eq!(&*field_name("status", &config), "status");
    }

    /// The reserved check runs against the *case-converted* name, so a
    /// keyword still collides under camelCase field naming (most keywords
    /// are already single lowercase words, which camelCase and snake_case
    /// both leave unchanged).
    #[test]
    fn test_field_name_mangles_reserved_word_under_camel_case() {
        let mut config = test_config();
        config.field_case = "camelCase".to_string();
        config.reserved = vec!["class".to_string()];
        assert_eq!(&*field_name("class", &config), "class_");
    }

    #[test]
    fn test_param_name_is_field_name_when_the_sql_name_is_already_an_identifier() {
        let config = test_config();
        assert_eq!(param_name("user_id", &config), field_name("user_id", &config));
    }

    #[test]
    fn test_param_name_replaces_characters_an_identifier_cannot_hold() {
        let config = test_config();
        assert_eq!(param_name("my col", &config), "my_col");
        assert_eq!(param_name("with-dash", &config), "with_dash");
        assert_eq!(param_name("it's", &config), "it_s");
    }

    /// The replacement runs before the case conversion, so `_` is already the
    /// word separator the converters split on. Running it after would leave
    /// `with-dash` unchanged under camelCase, because `to_snake_case` sees no
    /// boundary in it.
    #[test]
    fn test_param_name_replaces_characters_before_applying_the_case() {
        let mut config = test_config();
        config.field_case = "camelCase".to_string();
        assert_eq!(param_name("with-dash", &config), "withDash");
        assert_eq!(param_name("my col", &config), "myCol");
    }

    /// The guard is a word rather than a bare `_` so that it survives the
    /// case conversion, and therefore the second conversion a backend like
    /// go-pgx or csharp-npgsql applies on top.
    #[test]
    fn test_param_name_prefixes_a_leading_digit_under_every_case() {
        let mut config = test_config();
        assert_eq!(param_name("2fa", &config), "col_2fa");
        // The guard is added before the case runs, so it is cased along with
        // the rest of the name rather than sitting in front of it.
        config.field_case = "camelCase".to_string();
        assert_eq!(param_name("2fa", &config), "col2fa");
        config.field_case = "PascalCase".to_string();
        assert_eq!(param_name("2fa", &config), "Col2fa");
    }

    #[test]
    fn test_param_name_honours_both_reserved_lists() {
        let mut config = test_config();
        config.reserved = vec!["type".to_string()];
        config.reserved_bindings = vec!["class".to_string()];
        assert_eq!(param_name("type", &config), "type_");
        assert_eq!(param_name("class", &config), "class_");
        // The binding list is deliberately *not* consulted for a field: a
        // TypeScript row key named `class` is legal and must stay as the
        // driver spells it.
        assert_eq!(&*field_name("class", &config), "class");
    }

    #[test]
    fn test_enum_variant_name() {
        let config = test_config();
        assert_eq!(enum_variant_name("active", &config), "Active");
        assert_eq!(enum_variant_name("pending_review", &config), "PendingReview");
    }

    #[test]
    fn test_enum_variant_name_with_hyphens_and_dots() {
        let config = test_config();
        assert_eq!(enum_variant_name("gpt-3.5-turbo", &config), "Gpt35Turbo");
        assert_eq!(enum_variant_name("gpt-4-32k", &config), "Gpt432k");
        assert_eq!(
            enum_variant_name("command-light-nightly", &config),
            "CommandLightNightly"
        );
        assert_eq!(enum_variant_name("PG-13", &config), "Pg13");
        assert_eq!(enum_variant_name("NC-17", &config), "Nc17");
    }

    #[test]
    fn test_sanitize_for_identifier() {
        assert_eq!(sanitize_for_identifier("gpt-3.5-turbo"), "gpt_3_5_turbo");
        assert_eq!(sanitize_for_identifier("PG-13"), "PG_13");
        assert_eq!(sanitize_for_identifier("123abc"), "V123abc");
        assert_eq!(sanitize_for_identifier("normal_value"), "normal_value");
    }

    #[test]
    fn test_to_snake_case_consecutive_capitals() {
        assert_eq!(&*to_snake_case("HTTPClient"), "http_client");
        assert_eq!(&*to_snake_case("XMLParser"), "xml_parser");
        assert_eq!(&*to_snake_case("UserID"), "user_id");
        assert_eq!(&*to_snake_case("getHTTPSUrl"), "get_https_url");
        assert_eq!(&*to_snake_case("ABCDef"), "abc_def");
    }

    #[test]
    fn test_to_screaming_snake_case() {
        assert_eq!(&*to_screaming_snake_case("active"), "ACTIVE");
        assert_eq!(&*to_screaming_snake_case("user_status"), "USER_STATUS");
        assert_eq!(&*to_screaming_snake_case("PascalCase"), "PASCAL_CASE");
        assert_eq!(&*to_screaming_snake_case("pending_review"), "PENDING_REVIEW");
    }

    #[test]
    fn test_enum_variant_name_screaming_snake() {
        let config = NamingConfig {
            struct_case: "PascalCase".to_string(),
            fn_case: "snake_case".to_string(),
            enum_variant_case: "SCREAMING_SNAKE_CASE".to_string(),
            row_suffix: "Row".to_string(),
            field_case: "snake_case".to_string(),
            reserved: Vec::new(),
            reserved_bindings: Vec::new(),
            sanitize_field_names: false,
        };
        assert_eq!(enum_variant_name("active", &config), "ACTIVE");
        assert_eq!(enum_variant_name("pending_review", &config), "PENDING_REVIEW");
    }

    #[test]
    fn test_to_pascal_case_edge_cases() {
        assert_eq!(&*to_pascal_case("_user_status"), "UserStatus");
        assert_eq!(&*to_pascal_case("http_client"), "HttpClient");
    }

    /// This must fail before the fix: every `_`-delimited part being empty
    /// (a bare `"_"`, or the run of underscores `sanitize_for_identifier`
    /// leaves behind for a symbols-only value) made `to_pascal_case` return
    /// "" -- an empty type name reaches codegen as `pub enum {}`, which does
    /// not parse in any target language. `to_camel_case` already guards this
    /// exact shape (see `test_to_camel_case_underscore_edges`); this pins
    /// `to_pascal_case` agreeing with it instead of returning "".
    #[test]
    fn test_to_pascal_case_degenerate_underscores_do_not_collapse_to_empty() {
        assert_eq!(&*to_pascal_case("_"), "_");
        assert_eq!(&*to_pascal_case("__"), "__");
        assert_eq!(&*to_pascal_case("___"), "___");
    }

    /// Regression for #136: a schema-qualified enum's `.` must not reach the
    /// generated type name. Before the fix, `enum_type_name("public.status",
    /// ..)` under `struct_case = "PascalCase"` returned `"Public.status"` --
    /// `pub enum Public.status` does not parse in any target language, since
    /// `apply_case` alone never removes a character an identifier cannot
    /// hold.
    #[test]
    fn test_enum_type_name_sanitizes_a_schema_qualified_dot() {
        let config = test_config();
        assert_eq!(enum_type_name("public.status", &config), "PublicStatus");
    }

    /// The same schema-qualification defect under the other two
    /// `enum_variant_case`/`struct_case` conventions this crate's manifests
    /// actually declare (see `crates/scythe-codegen/manifests/*.toml`):
    /// Java/Kotlin/PHP/Python/Ruby use `SCREAMING_SNAKE_CASE` for variants,
    /// but every one of them still PascalCases the *type* name, so this
    /// defect was never gated by `enum_variant_case`.
    #[test]
    fn test_enum_type_name_sanitizes_a_schema_qualified_dot_regardless_of_variant_case() {
        let mut config = test_config();
        config.enum_variant_case = "SCREAMING_SNAKE_CASE".to_string();
        assert_eq!(enum_type_name("app.status", &config), "AppStatus");
    }

    /// A degenerate enum label under `PascalCase` -- the convention every
    /// C#, Go, Rust and TypeScript manifest declares for `enum_variant_case`
    /// -- must not collapse to an empty variant. Before the fix this printed
    /// an empty string: `sanitize_for_identifier("!!!")` is `"___"`, and the
    /// pre-fix `to_pascal_case("___")` returned "", so the generated file
    /// held a variant declared as nothing at all (`    ,` in Rust, `;` alone
    /// in C#) -- a syntax error in every one of those four targets.
    #[test]
    fn test_enum_variant_name_degenerate_label_does_not_collapse_to_empty() {
        let config = test_config();
        assert_eq!(enum_variant_name("!!!", &config), "___");
    }

    /// Under `SCREAMING_SNAKE_CASE` (Java/Kotlin/PHP/Python/Ruby) and
    /// `snake_case` (Elixir) the same degenerate label already produced a
    /// non-empty `"_"`/`"___"` before this fix -- `to_snake_case` does not
    /// collapse underscores the way `to_pascal_case` did. Pinned here so the
    /// PascalCase fix above is not mistaken for solving a problem those two
    /// conventions already avoided.
    #[test]
    fn test_enum_variant_name_degenerate_label_under_screaming_snake_case() {
        let mut config = test_config();
        config.enum_variant_case = "SCREAMING_SNAKE_CASE".to_string();
        assert_eq!(enum_variant_name("!!!", &config), "___");
    }
}
