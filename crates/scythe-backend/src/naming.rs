use std::borrow::Cow;

use serde::Deserialize;

/// Naming conventions for generated code.
#[derive(Debug, Clone, Deserialize)]
pub struct NamingConfig {
    pub struct_case: String,
    pub fn_case: String,
    pub enum_variant_case: String,
    pub row_suffix: String,
}

/// Convert a string to PascalCase.
///
/// Handles snake_case input ("user_status" -> "UserStatus")
/// and already-PascalCase input ("UserStatus" -> "UserStatus").
pub fn to_pascal_case(s: &str) -> Cow<'_, str> {
    let mut result = String::with_capacity(s.len());
    if s.contains('_') {
        for part in s.split('_') {
            let mut chars = part.chars();
            if let Some(c) = chars.next() {
                result.extend(c.to_uppercase());
                for ch in chars {
                    result.extend(ch.to_lowercase());
                }
            }
        }
    } else if let Some(first) = s.chars().next() {
        if first.is_lowercase() {
            let mut chars = s.chars();
            if let Some(first) = chars.next() {
                result.extend(first.to_uppercase());
                result.push_str(chars.as_str());
            }
        } else if s.chars().all(|c| c.is_uppercase() || c == '_') {
            let mut chars = s.chars();
            if let Some(first) = chars.next() {
                result.extend(first.to_uppercase());
                result.push_str(&chars.as_str().to_lowercase());
            }
        } else {
            return Cow::Borrowed(s);
        }
    } else {
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
/// Routes through [`to_snake_case`] first, then [`to_pascal_case`], then
/// lowercases the first character. `to_pascal_case` alone borrows unchanged
/// for mixed-case input with no underscore (e.g. "HTTPSUrl"), so calling it
/// directly on arbitrary input produced the broken "hTTPSUrl" instead of
/// "httpsUrl" — `to_snake_case` already has real consecutive-capital
/// handling, so normalizing through it first is what makes this the inverse
/// of `to_snake_case`.
pub fn to_camel_case(s: &str) -> Cow<'_, str> {
    let snake = to_snake_case(s);
    let pascal = to_pascal_case(&snake);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(c) => {
            let mut result = String::with_capacity(pascal.len());
            result.extend(c.to_lowercase());
            result.push_str(chars.as_str());
            Cow::Owned(result)
        }
        // Only reachable when `pascal` collapsed to nothing (e.g. an
        // all-underscore input like "__"), so the result is empty
        // regardless of what the original string held.
        None => Cow::Borrowed(""),
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
/// E.g., sql name "user_status" with PascalCase -> "UserStatus"
pub fn enum_type_name(sql_name: &str, naming: &NamingConfig) -> String {
    apply_case(sql_name, &naming.struct_case).into_owned()
}

/// Sanitize a string to be a valid Rust identifier fragment.
///
/// Replaces hyphens, dots, and other non-alphanumeric/non-underscore characters
/// with underscores, and prefixes with `V` if the result starts with a digit.
fn sanitize_for_identifier(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            result.push(ch);
        } else {
            result.push('_');
        }
    }
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
        assert_eq!(&*to_camel_case("__"), "");
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

    #[test]
    fn test_enum_type_name() {
        let config = test_config();
        assert_eq!(enum_type_name("user_status", &config), "UserStatus");
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
        };
        assert_eq!(enum_variant_name("active", &config), "ACTIVE");
        assert_eq!(enum_variant_name("pending_review", &config), "PENDING_REVIEW");
    }

    #[test]
    fn test_to_pascal_case_edge_cases() {
        assert_eq!(&*to_pascal_case("_user_status"), "UserStatus");
        assert_eq!(&*to_pascal_case("http_client"), "HttpClient");
    }
}
