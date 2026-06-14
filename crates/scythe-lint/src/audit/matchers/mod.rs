//! Named matcher functions for the built-in security rules.
//!
//! Each submodule exports a single `match_*` function that implements the
//! `MatcherFn` signature: `fn(&LintContext, &toml::Table) -> Vec<MatcherHit>`.

pub mod add_column_not_null_no_default;
pub mod add_constraint_without_using_index;
pub mod alter_column_drop_not_null;
pub mod alter_column_type;
pub mod alter_table_rename;
pub mod alter_table_rename_table;
pub mod cartesian_join;
pub mod column_type_disallowed;
pub mod constraint_missing_not_valid;
pub mod create_domain_with_constraint;
pub mod create_index_concurrency;
pub mod drop_statement;
pub mod function_name_in_set;
pub mod function_search_path_mutable;
pub mod grant_kind;
pub mod grantee_includes;
pub mod role_password_literal;
pub mod role_with_attribute;
pub mod security_definer_no_search_path;
pub mod select_star_over_pii_columns;
pub mod session_mutation;
pub mod truncate_cascade;
pub mod unbounded_pattern;
pub mod weak_hash_over_sensitive_column;

use super::registry::MatcherRegistry;

/// Register all canonical built-in matchers into `reg`.
pub fn register_canonical(reg: &mut MatcherRegistry) {
    // Security matchers (SC-SEC*)
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
    reg.register(
        "function_search_path_mutable",
        function_search_path_mutable::match_function_search_path_mutable,
    );

    // Migration matchers (SC-MIG*)
    reg.register("drop_statement", drop_statement::match_drop_statement);
    reg.register(
        "create_index_concurrency",
        create_index_concurrency::match_create_index_concurrency,
    );
    reg.register(
        "alter_table_rename_column",
        alter_table_rename::match_alter_table_rename_column,
    );
    reg.register(
        "constraint_missing_not_valid",
        constraint_missing_not_valid::match_constraint_missing_not_valid,
    );
    reg.register(
        "alter_table_rename_table",
        alter_table_rename_table::match_alter_table_rename_table,
    );
    reg.register("truncate_cascade", truncate_cascade::match_truncate_cascade);
    reg.register(
        "alter_column_type",
        alter_column_type::match_alter_column_type,
    );
    reg.register(
        "column_type_disallowed",
        column_type_disallowed::match_column_type_disallowed,
    );
    reg.register(
        "add_constraint_without_using_index",
        add_constraint_without_using_index::match_add_constraint_without_using_index,
    );
    reg.register(
        "create_domain_with_constraint",
        create_domain_with_constraint::match_create_domain_with_constraint,
    );
    reg.register(
        "alter_column_drop_not_null",
        alter_column_drop_not_null::match_alter_column_drop_not_null,
    );
    reg.register(
        "add_column_not_null_no_default",
        add_column_not_null_no_default::match_add_column_not_null_no_default,
    );
}
