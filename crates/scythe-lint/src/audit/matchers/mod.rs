//! Named matcher functions for the built-in security rules.
//!
//! Each submodule exports a single `match_*` function that implements the
//! `MatcherFn` signature: `fn(&LintContext, &toml::Table) -> Vec<MatcherHit>`.

pub mod cartesian_join;
pub mod function_name_in_set;
pub mod grant_kind;
pub mod grantee_includes;
pub mod role_password_literal;
pub mod role_with_attribute;
pub mod security_definer_no_search_path;
pub mod select_star_over_pii_columns;
pub mod session_mutation;
pub mod unbounded_pattern;
pub mod weak_hash_over_sensitive_column;

use super::registry::MatcherRegistry;

/// Register all eleven canonical built-in matchers into `reg`.
pub fn register_canonical(reg: &mut MatcherRegistry) {
    reg.register(
        "function_name_in_set",
        function_name_in_set::match_function_name_in_set,
    );
    reg.register("grant_kind", grant_kind::match_grant_kind);
    reg.register("grantee_includes", grantee_includes::match_grantee_includes);
    reg.register("cartesian_join", cartesian_join::match_cartesian_join);
    reg.register(
        "unbounded_pattern",
        unbounded_pattern::match_unbounded_pattern,
    );
    reg.register(
        "security_definer_no_search_path",
        security_definer_no_search_path::match_security_definer_no_search_path,
    );
    reg.register(
        "role_with_attribute",
        role_with_attribute::match_role_with_attribute,
    );
    reg.register(
        "role_password_literal",
        role_password_literal::match_role_password_literal,
    );
    reg.register(
        "weak_hash_over_sensitive_column",
        weak_hash_over_sensitive_column::match_weak_hash_over_sensitive_column,
    );
    reg.register(
        "select_star_over_pii_columns",
        select_star_over_pii_columns::match_select_star_over_pii_columns,
    );
    reg.register("session_mutation", session_mutation::match_session_mutation);
}
