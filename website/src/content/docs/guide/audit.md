---
title: Audit (security)
description: Run scythe's curated security rule set over SQL schema and queries.
---

`scythe audit` runs a curated set of security rules over your SQL schema and queries. It catches the kinds of issues that lint rules don't surface and that would otherwise only show up in a manual review or an incident: dangerous functions, over-broad GRANTs, cartesian joins, unbounded LIKE patterns, `SECURITY DEFINER` functions without a pinned `search_path`, role privilege escalation, literal passwords in DDL, weak hashes over credential columns, `SELECT *` over PII columns, and runtime session-state mutation.

The output is human-readable by default, and SARIF 2.1.0 or flat JSON for CI integration.

## Quick start

```bash
# Audit the SQL referenced by scythe.toml
scythe audit

# Audit one or more files directly
scythe audit migrations/*.sql

# Emit SARIF for GitHub code scanning
scythe audit --format sarif -o audit.sarif
```

Exit codes:

| Code | Meaning |
|------|---------|
| 0 | No error-severity findings, or `--exit-zero` was set |
| 1 | Configuration error (bad `scythe.toml`, missing files, malformed rule file) |
| 2 | One or more error-severity findings — distinct from `scythe lint` so CI can tell them apart |

## Rule catalog

The shipped rules span four prefixes: `SC-SEC*` (12 rules), `SC-RLS*` (3 rules), `SC-MIG*` (19
rules), and `SC-CHK01` (1 rule) — 35 rules total. Use `scythe audit --list-rules` to print the
current set with effective severities, and `scythe audit --explain <RULE_ID>` for the description
and CWE references of a specific rule.

### Security (`SC-SEC*`, category `security`)

| ID | Name | Severity | What it catches |
|----|------|----------|-----------------|
| SC-SEC01 | dangerous-function | error | Calls to functions that grant filesystem, network, or shell access (CWE-78) |
| SC-SEC02 | grant-all | error | `GRANT ALL` privilege widening (CWE-269) |
| SC-SEC03 | grant-to-public | error | `GRANT … TO PUBLIC` (CWE-269) |
| SC-SEC04 | superuser-role | error | `CREATE/ALTER ROLE` with SUPERUSER / CREATEROLE / similar high-privilege attributes (CWE-269) — Postgres only |
| SC-SEC05 | literal-password | error | Hard-coded literal password in `CREATE/ALTER ROLE` (CWE-798) — Postgres only |
| SC-SEC06 | weak-hash-in-auth | error | `md5()` / `sha1()` over credential-like columns (CWE-327, CWE-916) |
| SC-SEC07 | select-star-pii | warn | `SELECT *` against tables with PII or credential columns (CWE-200) |
| SC-SEC08 | cartesian-join | error | Unconstrained join producing a cartesian product (CWE-400) |
| SC-SEC09 | unbounded-like | warn | `LIKE '%…%'` — both-side wildcards, full-scan, DoS-prone (CWE-1333) |
| SC-SEC10 | security-definer-no-search-path | error | `SECURITY DEFINER` function without pinned `search_path` (CWE-426) — Postgres only |
| SC-SEC11 | session-mutation | error | `SET ROLE` / `SET SESSION AUTHORIZATION` / `RESET ROLE` inside application SQL (CWE-269) — Postgres only |
| SC-SEC12 | function-search-path-mutable | warn | `CREATE FUNCTION` without an explicit `SET search_path` (CWE-426) — Postgres only |

PG-only rules are no-ops on other dialects: when the dialect is not PostgreSQL they skip the AST entirely instead of producing false positives.

### Row Level Security (`SC-RLS*`, category `security`)

All three are Postgres-only.

| ID | Name | Severity | What it catches |
|----|------|----------|-----------------|
| SC-RLS01 | policy-references-user-metadata | error | RLS policy reads `user_metadata` (end-user-editable JWT claim) instead of `app_metadata` (CWE-639) |
| SC-RLS02 | policy-always-permissive | error | Permissive RLS policy with an always-true `USING`/`WITH CHECK` on a write command (CWE-285) |
| SC-RLS03 | policy-uses-uncached-auth-function | warn | RLS policy calls `auth.uid()`/`auth.jwt()`/`current_setting()` directly instead of `(select …)`, so it re-evaluates per row (CWE-405) |

### Migration safety (`SC-MIG*`, category `migration`)

The largest rule family scythe ships — 19 rules, all Postgres-only, all flagging DDL that is
irreversible, breaks a still-deployed application version, or takes a write-blocking lock.

| ID | Name | Severity | What it catches |
|----|------|----------|-----------------|
| SC-MIG01 | ban-drop-table | error | `DROP TABLE` — irreversible |
| SC-MIG02 | ban-drop-column | error | `ALTER TABLE … DROP COLUMN` — breaks deployed readers |
| SC-MIG03 | require-concurrent-index-creation | error | `CREATE INDEX` without `CONCURRENTLY` — `ACCESS EXCLUSIVE` for the build |
| SC-MIG04 | renaming-column | error | `ALTER TABLE … RENAME COLUMN` — breaks deployed readers |
| SC-MIG05 | constraint-missing-not-valid | error | `ADD CONSTRAINT` without `NOT VALID` — validates every row under `ACCESS EXCLUSIVE` |
| SC-MIG06 | ban-drop-database-or-schema | error | `DROP DATABASE`/`DROP SCHEMA` — destroys every contained object |
| SC-MIG07 | renaming-table | error | `ALTER TABLE … RENAME TO` — breaks deployed readers |
| SC-MIG08 | ban-truncate-cascade | error | `TRUNCATE … CASCADE` — clears every referencing table invisibly |
| SC-MIG09 | ban-alter-column-type | error | `ALTER COLUMN … TYPE` — rewrites the table under `ACCESS EXCLUSIVE` |
| SC-MIG10 | prefer-bigint-over-int | error | `int`/`integer`/`int4`/`smallint`/`int2` columns — 32-bit overflow risk |
| SC-MIG11 | prefer-text-over-varchar | error | `varchar(n)`/`character varying(n)`/`char(n)` — length increases are write-blocking |
| SC-MIG12 | prefer-timestamptz | error | `timestamp`/`timestamp without time zone` — silently shifts on session timezone |
| SC-MIG13 | prefer-identity-over-serial | error | `serial`/`bigserial`/`smallserial` — prefer `GENERATED ALWAYS AS IDENTITY` |
| SC-MIG14 | disallowed-unique-constraint | error | `ADD CONSTRAINT … UNIQUE (…)` — builds the index inline under `ACCESS EXCLUSIVE` |
| SC-MIG15 | adding-primary-key-constraint | error | `ADD CONSTRAINT … PRIMARY KEY (…)` — builds the index inline under `ACCESS EXCLUSIVE` |
| SC-MIG16 | ban-create-domain-with-constraint | error | `CREATE DOMAIN` with a `CHECK` — validates every row of every using table under `ACCESS EXCLUSIVE` |
| SC-MIG17 | ban-drop-not-null | error | `ALTER COLUMN … DROP NOT NULL` — relaxes a contract deployed code may rely on |
| SC-MIG18 | adding-not-nullable-field | warn | `ADD COLUMN … NOT NULL` without a `DEFAULT` — rewrites rows / breaks deployed writers |
| SC-MIG19 | unsupported-reg-types | error | `reg*` OID columns (other than `regclass`) — blocks `pg_upgrade`, doesn't survive dump/restore |

### Antipattern (`SC-CHK01`, category `antipattern`)

Despite living in the `quality.toml` source file, this rule's configured category is `antipattern`,
not `quality` — `[lint.categories]` keys off the configured category, so `[lint.categories]
antipattern = "off"` silences it, not a nonexistent `quality` key.

| ID | Name | Severity | What it catches |
|----|------|----------|-----------------|
| SC-CHK01 | check-constraint-always-true | warn | `CHECK` constraint expression is a tautology (`true`, `1 = 1`, …) — Postgres only |

## Suppression

A rule firing in one specific spot can be silenced with an inline annotation on the line above the offending statement:

```sql
-- scythe-audit: ignore[SC-SEC02] reason="security-reviewed: vetted role"
GRANT ALL ON internal_audit TO ops_admin;
```

Multiple rule IDs can be silenced on the same line by comma-separating them: `ignore[SC-SEC01,SC-SEC02]`. The annotation attaches to the next non-blank, non-comment line; only statements that begin on that line are exempt.

To run an audit *without* honouring any suppressions (useful for periodic strict scans), pass `--ignore-suppressions`.

## Severity filtering and exit codes

CI gates often want a graduated rollout: surface warnings, but only fail the build on errors. The default behaviour matches that.

| Want | Flag |
|------|------|
| Only see errors | `--severity error` |
| Surface warnings but never fail the build | `--exit-zero` |
| Block on any error finding | (default) |
| Treat the run as advisory | `--severity warn --exit-zero` |

## User-defined rules

Custom rules live in `scythe.toml` under `[audit]`, or in a separate TOML file referenced by `extra_rules`. Custom rule IDs must start with `USER-` to avoid collisions with shipped rules. (The
equivalent for `scythe inspect`'s live-DB checks requires the longer `USER-INS-` prefix — see
[Inspect](/scythe/guide/inspect/).)

```toml
[audit]
extra_rules = ["./security_rules.toml"]

[[audit.rule]]
id = "USER-001"
name = "no-debug-functions"
category = "security"
severity = "error"
description = "calls to debug-only functions should not ship"
message = "call to debug function `{func}` — remove before merging"
matcher = "function_name_in_set"

[audit.rule.matcher_args]
functions = ["dump_internal_state", "debug_print"]
```

The `matcher` field references one of the in-tree matchers. Run `scythe audit --list-rules` after editing `scythe.toml` to confirm your rule is picked up.

### Available matchers

All 28 matchers scythe registers, with the `matcher_args` each one reads:

| Matcher | Required `matcher_args` |
|---------|-------------------------|
| `function_name_in_set` | `functions = ["fn1", "fn2", ...]` |
| `grant_kind` | `kind = "all"` |
| `grantee_includes` | `grantee = "public"` |
| `cartesian_join` | -- |
| `unbounded_pattern` | -- |
| `security_definer_no_search_path` | -- |
| `role_with_attribute` | `attributes = ["superuser", "createdb", ...]` |
| `role_password_literal` | -- |
| `weak_hash_over_sensitive_column` | `functions = ["md5", "sha1"]`, `column_patterns = ["password", ...]` |
| `select_star_over_pii_columns` | `column_patterns = ["password", "ssn", ...]` |
| `session_mutation` | `kinds = ["set_role", "set_session_authorization", "reset_role"]` |
| `function_search_path_mutable` | -- |
| `policy_references_user_metadata` | -- |
| `policy_always_permissive` | -- |
| `policy_uses_uncached_auth_function` | -- |
| `drop_statement` | `kinds = ["table"]` (or `"column"`, `"database"`, `"schema"`) |
| `create_index_concurrency` | -- |
| `alter_table_rename_column` | -- |
| `constraint_missing_not_valid` | -- |
| `alter_table_rename_table` | -- |
| `truncate_cascade` | -- |
| `alter_column_type` | -- |
| `column_type_disallowed` | `disallowed = ["int", "integer", ...]`, `suggested = "bigint"` |
| `add_constraint_without_using_index` | `kinds = ["unique"]` (or `"primary_key"`) |
| `create_domain_with_constraint` | -- |
| `alter_column_drop_not_null` | -- |
| `add_column_not_null_no_default` | -- |
| `check_constraint_always_true` | -- |

## CI integration

### GitHub Actions — SARIF upload

```yaml
- name: Run scythe audit
  run: scythe audit --format sarif -o audit.sarif --exit-zero
- name: Upload SARIF
  uses: github/codeql-action/upload-sarif@v3
  with:
    sarif_file: audit.sarif
```

`--exit-zero` keeps the job green so the SARIF upload always runs; GitHub Code Scanning still surfaces findings in the PR.

### GitLab CI — SAST report

```yaml
audit:
  image: rust:latest
  script:
    - cargo install scythe-cli
    - scythe audit --format json -o gl-sast-report.json --exit-zero
  artifacts:
    reports:
      sast: gl-sast-report.json
```

### Pre-commit — block on errors

Use the public hooks published by this repo:

```yaml
- repo: https://github.com/Goldziher/scythe
  rev: v0.16.1              # pin to a released tag
  hooks:
    - id: scythe-audit     # SC-SEC*/SC-RLS*/SC-MIG*/SC-CHK* on changed files
    # - id: scythe-lint    # full pipeline: sqruff + scythe-lint + audit (needs scythe.toml)
```

`scythe-audit` runs on every staged `.sql` file with the default postgres dialect.
Override per-hook via the standard pre-commit `args:` block:

```yaml
- id: scythe-audit
  args: [--dialect, mysql, --severity, warn]
```

`scythe lint` already invokes the audit rule pack with dialect gating —
rules whose `dialects` list excludes the configured `[[sql]].engine` are
silently skipped, so a `mysql` project will not see SC-MIG* (postgres-only)
findings. The `scythe-lint` hook is the right choice when a `scythe.toml`
is present at the repo root; `scythe-audit` covers projects that don't (yet)
use scythe for codegen.

By default `scythe audit` exits 2 on error findings, which pre-commit treats as a failed hook.
For advisory CI integration that publishes findings without blocking, add `--exit-zero`.
