//! `MatcherRegistry` — maps matcher names to `MatcherFn` implementations.

use ahash::AHashMap;

use crate::types::LintContext;

/// Bindings produced by a matcher on a hit.  Keys match the `{var}`
/// placeholders used in the rule's message template.
#[derive(Debug, Default)]
pub struct MatcherHit {
    pub bindings: AHashMap<String, String>,
}

impl MatcherHit {
    /// Construct a hit with no bindings (for rules whose messages need none).
    pub fn empty() -> Self {
        Self {
            bindings: AHashMap::new(),
        }
    }

    /// Construct a hit with a single binding.
    pub fn with_binding(key: impl Into<String>, value: impl Into<String>) -> Self {
        let mut bindings = AHashMap::new();
        bindings.insert(key.into(), value.into());
        Self { bindings }
    }
}

/// A matcher function: takes a `LintContext` and opaque per-rule args from the
/// TOML `[rule.matcher_args]` table.  Returns zero or more hits, each carrying
/// string bindings used to render the rule's message template.
pub type MatcherFn = fn(&LintContext<'_>, &toml::Table) -> Vec<MatcherHit>;

// ---------------------------------------------------------------------------
// MatcherRegistry
// ---------------------------------------------------------------------------

/// Registry mapping matcher names to [`MatcherFn`] implementations.
pub struct MatcherRegistry {
    matchers: AHashMap<&'static str, MatcherFn>,
}

impl MatcherRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            matchers: AHashMap::new(),
        }
    }

    /// Register a named matcher.
    pub fn register(&mut self, name: &'static str, f: MatcherFn) {
        self.matchers.insert(name, f);
    }

    /// Look up a matcher by name.
    pub fn get(&self, name: &str) -> Option<MatcherFn> {
        self.matchers.get(name).copied()
    }

    /// Build the canonical registry with all eleven built-in matchers wired.
    pub fn canonical() -> Self {
        let mut reg = Self::new();
        super::matchers::register_canonical(&mut reg);
        reg
    }
}

impl Default for MatcherRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_matcher(_ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
        vec![MatcherHit::empty()]
    }

    #[test]
    fn register_and_get_returns_fn() {
        let mut reg = MatcherRegistry::new();
        reg.register("test-matcher", dummy_matcher);
        let got = reg.get("test-matcher");
        assert!(got.is_some());
    }

    #[test]
    fn get_unknown_returns_none() {
        let reg = MatcherRegistry::new();
        assert!(reg.get("does-not-exist").is_none());
    }

    #[test]
    fn matcher_hit_empty() {
        let hit = MatcherHit::empty();
        assert!(hit.bindings.is_empty());
    }

    #[test]
    fn matcher_hit_with_binding() {
        let hit = MatcherHit::with_binding("func", "pg_read_file");
        assert_eq!(
            hit.bindings.get("func").map(|s| s.as_str()),
            Some("pg_read_file")
        );
    }

    #[test]
    fn canonical_registry_has_all_matchers() {
        let reg = MatcherRegistry::canonical();
        let expected = [
            // Security
            "function_name_in_set",
            "grant_kind",
            "grantee_includes",
            "cartesian_join",
            "unbounded_pattern",
            "security_definer_no_search_path",
            "role_with_attribute",
            "role_password_literal",
            "weak_hash_over_sensitive_column",
            "select_star_over_pii_columns",
            "session_mutation",
            // Migration
            "drop_statement",
            "create_index_concurrency",
            "alter_table_rename_column",
            "constraint_missing_not_valid",
            "alter_table_rename_table",
            "truncate_cascade",
            "alter_column_type",
        ];
        for name in &expected {
            assert!(
                reg.get(name).is_some(),
                "canonical registry missing matcher: {name}"
            );
        }
    }
}
