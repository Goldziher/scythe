use std::borrow::Cow;

use serde::Deserialize;

/// Naming conventions for generated code.
#[derive(Debug, Clone, Deserialize)]
pub struct NamingConfig {
    pub struct_case: String,
    pub field_case: String,
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
            // Single word, lowercase: capitalize first letter
            let mut chars = s.chars();
            if let Some(first) = chars.next() {
                result.extend(first.to_uppercase());
                result.push_str(chars.as_str());
            }
        } else {
            // Already PascalCase or single uppercase word
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

    // Check if already all lowercase with no uppercase
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
        None => Cow::Borrowed(s),
    }
}

/// Apply a named case convention to a string.
pub fn apply_case<'a>(s: &'a str, case: &str) -> Cow<'a, str> {
    match case {
        "PascalCase" => to_pascal_case(s),
        "snake_case" => to_snake_case(s),
        "camelCase" => to_camel_case(s),
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

/// Generate an enum variant name from its SQL value.
///
/// E.g., sql value "active" with PascalCase -> "Active"
pub fn enum_variant_name(sql_value: &str, naming: &NamingConfig) -> String {
    apply_case(sql_value, &naming.enum_variant_case).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> NamingConfig {
        NamingConfig {
            struct_case: "PascalCase".to_string(),
            field_case: "snake_case".to_string(),
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
        // Already PascalCase should return Cow::Borrowed
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
        // Already snake_case should return Cow::Borrowed
        assert!(matches!(to_snake_case("user_status"), Cow::Borrowed(_)));
    }

    #[test]
    fn test_to_camel_case() {
        assert_eq!(&*to_camel_case("user_status"), "userStatus");
        assert_eq!(&*to_camel_case("UserStatus"), "userStatus");
        assert_eq!(&*to_camel_case("get_user"), "getUser");
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
        assert_eq!(
            enum_variant_name("pending_review", &config),
            "PendingReview"
        );
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
    fn test_to_pascal_case_edge_cases() {
        assert_eq!(&*to_pascal_case("_user_status"), "UserStatus");
        assert_eq!(&*to_pascal_case("http_client"), "HttpClient");
    }
}
