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

Detection patterns from splinter lints that cannot be ported because they
require live catalog state (`pg_stat_user_indexes`, `pg_policies`, role
grants, `pg_depend`, etc.) are tracked but not implemented in scythe; users
who want those checks should run splinter directly against a deployed
database.
