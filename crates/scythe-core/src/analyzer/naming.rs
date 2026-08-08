//! Case-conversion helpers for the phase-2 nested-struct naming pass
//! (`analyzer/mod.rs::analyze`).
//!
//! This deliberately duplicates the algorithm in `scythe-backend`'s
//! `naming::to_pascal_case`/`to_snake_case` rather than depending on that
//! crate: `scythe-backend` depends on `scythe-core`, so the reverse
//! dependency would be circular. The two implementations should stay in
//! sync; a shared `scythe-naming` crate both could depend on is a
//! reasonable follow-up if a third caller ever needs the same algorithm.

use std::borrow::Cow;

/// Convert a string to PascalCase.
///
/// Handles snake_case input (`"user_status"` -> `"UserStatus"`) and
/// already-PascalCase input (`"UserStatus"` -> `"UserStatus"`).
pub(super) fn to_pascal_case(s: &str) -> Cow<'_, str> {
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
/// Handles PascalCase input (`"UserStatus"` -> `"user_status"`) and
/// already-snake_case input (`"user_status"` -> `"user_status"`). Correctly
/// handles consecutive uppercase letters: `"HTTPClient"` -> `"http_client"`,
/// `"UserID"` -> `"user_id"`.
pub(super) fn to_snake_case(s: &str) -> Cow<'_, str> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(&*to_pascal_case("get_user_orders_row_orders"), "GetUserOrdersRowOrders");
        assert_eq!(&*to_pascal_case("UserStatus"), "UserStatus");
        assert_eq!(&*to_pascal_case("active"), "Active");
    }

    #[test]
    fn test_to_snake_case() {
        assert_eq!(&*to_snake_case("GetUserOrders"), "get_user_orders");
        assert_eq!(&*to_snake_case("user_status"), "user_status");
        assert_eq!(&*to_snake_case("HTTPClient"), "http_client");
    }

    /// The phase-2 naming pass relies on `to_pascal_case` alone to derive
    /// the form embedded in a column's `neutral_type` from
    /// `NestedStructInfo.name` — never a `to_snake_case(to_pascal_case(x))`
    /// round trip, which is lossy for a collision-suffixed name (`_1`
    /// becomes `1` with no digit/word boundary to reinsert the underscore
    /// at). Pin the forward direction, which is the only one the resolver
    /// in `analyzer/mod.rs` actually performs.
    #[test]
    fn test_pascal_case_forward_direction_for_suffixed_names() {
        assert_eq!(&*to_pascal_case("get_user_orders_row_orders"), "GetUserOrdersRowOrders");
        assert_eq!(&*to_pascal_case("list_posts_row_comments_1"), "ListPostsRowComments1");
    }
}
