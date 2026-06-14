# Attributions

scythe is licensed under MIT (see `LICENSE`). This file lists external projects
whose ideas or detection patterns informed scythe's design, even when no source
code was copied.

## Inspirations

### supabase/splinter

Detection patterns for several `scythe audit` Postgres rules are inspired by
the runtime lint catalog in [supabase/splinter](https://github.com/supabase/splinter).
Splinter runs its lints as SELECT queries against `pg_catalog` and
`information_schema` on a live database; scythe runs the equivalent
detections at lint time against the `sqlparser` AST of a migration script.

**No source code is copied from splinter.** Each rule is a clean-room
reimplementation of the published detection logic. The splinter repository
does not carry a LICENSE file at the time of writing (verified
2026-06-14 via `gh api repos/supabase/splinter/license` → 404; the README's
LICENSE link is broken). Under default copyright law, all rights to the
splinter source code are reserved by Supabase, Inc.; this attribution is
courtesy, not a legal requirement, and citing it does not relicense any
splinter code.

Rules inspired by splinter lints:

| scythe rule | splinter lint | Adaptation |
|---|---|---|
| SC-SEC12 function-search-path-mutable | 0011 function_search_path_mutable | Lint-time check on `CreateFunction.set_params` instead of runtime `pg_proc.proconfig`. Complementary to existing SC-SEC10 (which owns the escalating `SECURITY DEFINER` case at `error`); SC-SEC12 covers the general invoker case at `warn`. |
| SC-MIG19 unsupported-reg-types | 0018 unsupported_reg_types | Lint-time check on `CREATE TABLE` / `ALTER TABLE ADD COLUMN` column types instead of runtime `pg_attribute` scan. Same banned-type set (regcollation, regconfig, regdictionary, regnamespace, regoper, regoperator, regproc, regprocedure); regclass is exempt. |
| SC-RLS01 policy-references-user-metadata | 0015 rls_references_user_metadata | AST walk of `CreatePolicy.using` / `.with_check` for substring `user_metadata`, instead of runtime `pg_policies.qual` regex. |
| SC-RLS02 policy-always-permissive | 0024 rls_policy_always_true | Typed-AST tautology detection over `Expr::Value(Boolean(true))`, `Expr::Value(Null)`, and `BinaryOp Eq(Value, Value)` with matching operands, instead of runtime string normalisation. Same exclusions: SELECT-only policies and RESTRICTIVE policies are not flagged. |
| SC-RLS03 policy-uses-uncached-auth-function | 0003 auth_rls_initplan | Typed-AST walk of `CreatePolicy.using` / `.with_check` for `Expr::Function` calls to `auth.uid` / `auth.jwt` / `auth.role` / `auth.email` / `current_setting`, stopping at `Expr::Subquery` boundaries (the `(select …)` wrapping is the safe form). Splinter uses substring matching with negative-pattern carve-outs for the wrapped form. |

Detection patterns from splinter lints that cannot be ported to the static
audit pipeline because they require live catalog state moved into the
`scythe inspect` command instead. Splinter remains the closest prior art for
those — scythe-inspect carries forward the highest-impact subset.

#### Live inspection rules inspired by splinter (scythe-inspect)

| scythe rule | splinter lint | Adaptation |
|---|---|---|
| SC-INS01 missing-fk-index | 0001 unindexed_foreign_keys | Runtime `pg_constraint` + `pg_index` join, grouping FK columns and asserting a covering index on the leading column set. Splinter's same shape; `tokio-postgres` parameterless query at lint time. |
| SC-INS02 policy-exists-rls-disabled | 0006 policy_exists_rls_disabled | `pg_class` + `pg_policy` join filtering `NOT relrowsecurity`. Splinter's same shape. |
| SC-INS03 duplicate-index | 0009 duplicate_index | `pg_indexes` group-by on `regexp_replace`-normalised `indexdef`, HAVING `count(*) > 1`. Splinter's same shape. |

The remaining splinter lints that need live catalog state — auth/RLS deep
dives, `pg_stat_*`-based unused-index and slow-query detection, extension
placement checks — are tracked for later scythe-inspect phases (see the
phased roadmap in `docs/guide/inspect.md`).
