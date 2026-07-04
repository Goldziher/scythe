# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.11.0] - 2026-07-04

### Added

- Full `:grouped` / `@group_by` nested code generation across every backend. A `:grouped` query now emits a child struct plus a parent struct carrying a `children` collection, and a query function that runs the flat SQL and folds rows into an order-preserving list of parents keyed by the grouping column — all client-side, with the SQL unchanged from `:many`. Previously `:grouped` silently degraded to a flat `:many` proxy despite the docs promising nesting (#55). Implemented for all Rust, Python, TypeScript, C#, Go, Ruby, PHP, Elixir, and Java/Kotlin backends, each with language-native structs, collection types, and fold idioms.
- `CodegenBackend::generate_grouped_structs` and `generate_grouped_query_fn` trait methods (inputs bundled in a `GroupedQueryFn` context struct) with default implementations that return a clear "grouped queries are not yet supported by '<backend>'" error, so future backends opt in incrementally without panicking.
- Positional param-naming escape hatch: `-- @param $N <name>[: <description>]` overrides the inferred/`pN` fallback name for a placeholder by position, flowing the chosen name to every language. The existing docs-only `-- @param <name>: <description>` form is unchanged (#53).
- Lint rule SC-S07 `unbound-sql-param` (error): flags any `$N` present in the SQL body but absent from the generated parameter signature, backstopping the whole class of silent param drops.

### Fixed

- Params inside a FROM-clause derived table (subquery) are no longer discarded — the sub-analyzer's collected params and positional counter are merged back into the parent scope (#52, Case C).
- Placeholders nested inside an `UPDATE … SET` arithmetic expression such as `SET credits = credits + $2` are now collected instead of silently dropped; param collection recurses through `BinaryOp`/`UnaryOp`/`Nested` expressions (but not subqueries, which own their own param scope). Caught by SC-S07 (#52).
- Unsupported inline named placeholders (`:name`) now fail fast with a query-pointed error instead of emitting broken codegen (#52, Cases A/B).

### Changed

- Workspace crate versions bumped 0.10.0 → 0.11.0 across all six crates, with cross-crate path-dep version pins updated.
- `sqruff-lib` upgraded 0.38 → 0.39 (`cargo upgrade --incompatible`); lockfile refreshed.

## [0.10.0] - 2026-06-14

### Added

- `scythe inspect <database-url>` subcommand — live-database operational health checks. Connects via `tokio-postgres` and runs a set of `pg_catalog` queries that detect issues only visible in a running database, then emits findings in the same human / SARIF 2.1.0 / JSON reporter shapes used by `scythe audit`. URL resolution: positional argument, then `$DATABASE_URL`, then `$SCYTHE_DATABASE_URL`. Builds a per-invocation `tokio::runtime::Builder::new_current_thread()` runtime so the rest of the CLI (`lint`, `audit`, `generate`) stays synchronous.
- New `scythe-inspect` crate (`crates/scythe-inspect/`) carrying a `DbDriver` async trait, a `PostgresDriver` implementation backed by `tokio-postgres`, and a `MysqlDriver` stub that returns `InspectError::Unsupported("mysql")` from `connect` and `run_all`. The stub exists to keep the trait shape engine-agnostic; a real MySQL driver lands in Phase 3 (v0.13.0).
- Three Postgres operational checks at Phase 0, clean-room reimplemented from the equivalent supabase/splinter lints (no source code copied; ATTRIBUTIONS.md updated): SC-INS01 missing-fk-index (warn — foreign-key columns with no covering index force a sequential scan on every join through the constraint; splinter 0001), SC-INS02 policy-exists-rls-disabled (error — table has `CREATE POLICY` definitions but `ROW LEVEL SECURITY` is disabled, so the policies never apply; splinter 0006), and SC-INS03 duplicate-index (warn — two or more indexes with identical definitions modulo name; splinter 0009).
- `scythe inspect --list-checks` prints the check catalog (id, name, severity, description) without connecting, so users can discover the rule set offline.
- `scythe inspect --format <human|sarif|json>`, `--severity <off|warn|error>`, `--exit-zero`, `--output <PATH>`, `--dialect <postgres|mysql>` — mirror the audit subcommand surface for consistency. Exit code 2 on remaining error-severity findings unless `--exit-zero` is set; exit 0 otherwise. Severity floor filtering applies before emission.
- Public `scythe-inspect` pre-commit hook published via `.pre-commit-hooks.yaml`. CI-mode hook: `always_run: true`, `pass_filenames: false`, requires `$DATABASE_URL` (or `$SCYTHE_DATABASE_URL`) in the hook environment. Local pre-commit runs without the variable fail loudly with the same error as the CLI. Phase 1 (v0.11.0) will add `scythe.toml` `[inspect]` URL sourcing so local use becomes natural.
- New documentation page `docs/guide/inspect.md` covering quick-start, check catalog, severity/exit-code semantics, GitHub Actions CI recipe with `services: postgres`, pre-commit usage, what `scythe inspect` does not do (yet), and the phased roadmap through v0.14.0.
- `docs/guide/cli-reference.md` extended with the `inspect` subcommand and every flag; `docs/guide/pre-commit-hooks.md` adds the new `scythe-inspect` hook row and section; README adds `scythe inspect` to the feature list and a Documentation link.
- New CI workflow `.github/workflows/inspect-live.yml` spins up `postgres:16-alpine` as a service and runs `cargo test -p scythe-inspect --features live-tests`. Triggered on PRs that touch `crates/scythe-inspect/**`. Default `cargo test` runs stay DB-free.
- `ATTRIBUTIONS.md` extended with a "Live inspection rules inspired by splinter (scythe-inspect)" subsection citing splinter lints 0001, 0006, 0009 against SC-INS01, SC-INS02, SC-INS03 respectively.

### Changed

- Workspace crate versions bumped 0.9.0 → 0.10.0 across all six crates (the five existing crates plus the new `scythe-inspect`), with cross-crate path-dep version pins updated.
- `scythe lint` and `scythe audit` are unaffected — Phase 0 adds the inspect surface without touching the static pipeline.

## [0.9.0] - 2026-06-14

### Added

- `scythe audit` subcommand — static security analyzer for SQL. Reads `.sql` files, runs a built-in security rule pack, and emits findings as human-readable text, SARIF 2.1.0 (with CWE tags for code-scanning ingest), or JSON. Exits non-zero when any rule fires, so it slots into CI gates.
- `scythe audit --list-rules` — print the rule catalog (id, name, severity, category, description) grouped by category, then exit 0. Reflects user-loaded rules from `scythe.toml` so the catalog is honest.
- `scythe audit --explain <RULE_ID>` — print the description and CWE references for a rule by id, then exit 0. Useful for figuring out why a rule fired without going to the docs.
- `scythe audit --severity <off|warn|error>` — drop findings below the given level so CI gates can graduate from warnings to errors.
- `scythe audit --exit-zero` — always exit 0 after emitting findings, for advisory CI integrations that publish findings but don't gate the build.
- `scythe audit -o, --output <PATH>` — write reporter output to a file instead of stdout. Useful for SARIF/JSON artifacts in CI.
- `scythe audit --ignore-suppressions` — disable inline `-- scythe-audit: ignore[...]` annotations for periodic strict scans.
- `scythe audit --dialect <postgres|mysql|sqlite|mssql|oracle|snowflake>` — set the SQL dialect for explicit-file mode (config mode already inherits the dialect from `[[sql]].engine`).
- New docs page `docs/guide/audit.md` covering quick-start, rule catalog, suppression syntax, user-defined rules, available matchers, and CI integration recipes (GitHub Actions SARIF, GitLab SAST, pre-commit). `docs/guide/cli-reference.md` extended with the `audit` subcommand and every flag.
- `Severity` now derives `PartialOrd`/`Ord` and gains a `Severity::parse_cli` helper so CLI consumers can resolve `off`/`warn`/`error` to a typed minimum.
- Eleven canonical security rules ship in `scythe-lint`'s `audit` module: SC-SEC01 dangerous-function (CWE-78), SC-SEC02 grant-all (CWE-269), SC-SEC03 grant-to-public (CWE-269), SC-SEC04 superuser-role (CWE-269) covering SUPERUSER/CREATEDB/CREATEROLE/REPLICATION/BYPASSRLS, SC-SEC05 literal-password (CWE-798), SC-SEC06 weak-hash-in-auth (CWE-327, CWE-916), SC-SEC07 select-star-pii (CWE-200), SC-SEC08 cartesian-join (CWE-400), SC-SEC09 unbounded-like (CWE-1333), SC-SEC10 security-definer-no-search-path (CWE-426), and SC-SEC11 session-mutation (CWE-269) covering SET ROLE / SET SESSION AUTHORIZATION / RESET ROLE.
- Hybrid matcher framework: rule metadata lives in TOML, AST-matching logic lives in named Rust functions registered against a `MatcherRegistry`. Adding a rule that reuses an existing matcher is now a TOML stanza, not a Rust file. Canonical rules ship in-tree via `include_str!` so the default registry has zero runtime config dependencies.
- User-defined audit rules via `scythe.toml`: `[[audit.rule]]` for inline rules and `extra_rules = ["./path.toml"]` to load separate files. IDs must start with `USER-`; collisions with canonical `SC-SEC*` IDs are rejected at load time with the offending ID and source path.
- Inline suppressions: `-- scythe-audit: ignore[SC-SEC02,SC-SEC09] reason="vetted"` attaches to the next statement and suppresses the listed rule IDs for every line of that statement (terminated by a blank line or `;`). Reason clauses are parsed and discarded. Malformed annotations are silently ignored.
- `LintContext.dialect: SqlDialect` field, threaded through every rule call site, so matchers can dialect-filter via `dialects = [...]` in the rule spec.
- `RuleFile` TOML schema with `schema_version = 1` for forward-compatible rule files.
- New `migration` rule category and nine canonical migration-safety rules under the `SC-MIG*` prefix: SC-MIG01 ban-drop-table, SC-MIG02 ban-drop-column, SC-MIG03 require-concurrent-index-creation, SC-MIG04 renaming-column, SC-MIG05 constraint-missing-not-valid, SC-MIG06 ban-drop-database-or-schema, SC-MIG07 renaming-table, SC-MIG08 ban-truncate-cascade, SC-MIG09 ban-alter-column-type. Each rule targets a class of irreversible or lock-prone Postgres DDL change that breaks zero-downtime deployments. All declare `dialects = ["postgres"]`. Seven matcher functions back them: `drop_statement` (parameterised by `kinds = ["table", "column", "database", "schema"]` so a single matcher serves SC-MIG01/SC-MIG02/SC-MIG06), `create_index_concurrency`, `alter_table_rename_column`, `constraint_missing_not_valid`, `alter_table_rename_table`, `truncate_cascade`, `alter_column_type`. The matcher framework is unchanged.
- Four additional column-type-preference migration rules backed by a single new `column_type_disallowed` matcher: SC-MIG10 prefer-bigint-over-int (fires on `int`/`integer`/`int4`/`smallint`/`int2` — 32-bit keys overflow at 2^31 and widening requires a write-blocking ALTER), SC-MIG11 prefer-text-over-varchar (fires on `varchar(n)`/`character varying(n)`/`char(n)` — Postgres stores these identically to `text`; a length bump is write-blocking), SC-MIG12 prefer-timestamptz (fires on `timestamp`/`timestamp without time zone` — naive timestamps silently shift on session timezone changes), SC-MIG13 prefer-identity-over-serial (fires on `serial`/`bigserial`/`smallserial` — SERIAL is legacy implicit-sequence shorthand; `GENERATED AS IDENTITY` is the SQL-standard replacement). The matcher walks `CREATE TABLE` columns and `ALTER TABLE … ADD COLUMN` operations, using exact-match and prefix-before-`(` semantics to avoid false-positives (e.g. `bigint` does not fire when `int` is disallowed). Emits `table`, `column`, `actual_type`, and `suggested_type` bindings.
- The `scythe audit` dispatcher now also runs rules in the new `migration` category; `--list-rules` groups SC-MIG* under a separate `[migration]` heading.
- Three additional constraint-lock migration rules covering the next class of Squawk-derived ALTER hazards: SC-MIG14 disallowed-unique-constraint (fires on `ALTER TABLE … ADD CONSTRAINT … UNIQUE (…)` — builds the index inline under ACCESS EXCLUSIVE; safe pattern is `CREATE UNIQUE INDEX CONCURRENTLY` followed by `ADD CONSTRAINT … UNIQUE USING INDEX`), SC-MIG15 adding-primary-key-constraint (fires on `ALTER TABLE … ADD CONSTRAINT … PRIMARY KEY (…)` — same lock hazard, same `USING INDEX` workaround), SC-MIG16 ban-create-domain-with-constraint (fires on `CREATE DOMAIN … CHECK (…)` — Postgres validates every row of every table using the domain under ACCESS EXCLUSIVE and the constraint cannot be split into `NOT VALID` + `VALIDATE`). Two new matchers back them: `add_constraint_without_using_index` (parameterised by `kinds = ["unique", "primary_key"]` so a single matcher serves SC-MIG14/SC-MIG15, and distinguishes the plain `UNIQUE`/`PRIMARY KEY` table constraints from the `… USING INDEX` variants) and `create_domain_with_constraint`.
- Two NULL-contract-integrity migration rules: SC-MIG17 ban-drop-not-null (error — fires on `ALTER TABLE … ALTER COLUMN … DROP NOT NULL`; relaxing a NOT NULL contract breaks deployed application versions and ORM mappings that still treat the column as non-null) and SC-MIG18 adding-not-nullable-field (warn — fires on `ALTER TABLE … ADD COLUMN … NOT NULL` without a `DEFAULT`; rewrites every existing row on Postgres <11 and breaks deployed application versions that insert without the new column). Two new matchers back them: `alter_column_drop_not_null` and `add_column_not_null_no_default`. Both rules declare `dialects = ["postgres"]`.
- Two splinter-inspired rules covering function search-path hygiene and pg_upgrade-blocking column types: SC-SEC12 function-search-path-mutable (warn — fires on `CREATE FUNCTION` without `SET search_path = …` and not `SECURITY DEFINER`; complementary to SC-SEC10 which owns the escalating DEFINER case at error severity, so the two rules never double-count on the same statement) and SC-MIG19 unsupported-reg-types (error — fires when a column type is `regcollation`/`regconfig`/`regdictionary`/`regnamespace`/`regoper`/`regoperator`/`regproc`/`regprocedure`; reg* OID types other than `regclass` block `pg_upgrade` and do not survive logical dump/restore). One new matcher (`function_search_path_mutable`); SC-MIG19 reuses the existing `column_type_disallowed` matcher with an empty `suggested` and a regtype `disallowed` list. Detection patterns inspired by supabase/splinter lints 0011 and 0018 — see `ATTRIBUTIONS.md`.
- `ATTRIBUTIONS.md` at the repo root listing external projects whose detection patterns informed scythe rules. Initial entry credits supabase/splinter and documents the no-license caveat (clean-room reimplementation only).
- Row Level Security rule pack — three rules under the new `SC-RLS*` prefix (still `category = "security"`): SC-RLS01 policy-references-user-metadata (error, CWE-639 — fires on `CREATE POLICY` whose USING or WITH CHECK reads from `user_metadata`, an end-user-editable JWT claim; safe path uses server-set `app_metadata`), SC-RLS02 policy-always-permissive (error, CWE-285 — fires on a permissive policy whose USING or WITH CHECK is a tautology like `(true)`, `(1 = 1)`, or `NULL` on a write-side command; SELECT policies and restrictive policies are excluded), SC-RLS03 policy-uses-uncached-auth-function (warn, CWE-405 — fires on a bare `auth.uid()` / `auth.jwt()` / `auth.role()` / `auth.email()` / `current_setting(…)` call in the policy expression without wrapping in a scalar subquery; wrapping lets Postgres cache the result as an InitPlan instead of re-evaluating per row). Three new matchers walk the typed `CreatePolicy.using` / `.with_check` `Expr` ASTs. SC-RLS03 specifically stops at `Expr::Subquery` boundaries — that's the safe form. Detection patterns inspired by supabase/splinter lints 0015, 0024, 0003 — see `ATTRIBUTIONS.md`.
- CHECK-constraint quality rule SC-CHK01 check-constraint-always-true (warn, `category = "antipattern"`): fires when a CHECK constraint expression is a tautology (`true`, `1 = 1`, `NULL`, parenthesised variants). Covers column-level CHECK in `CREATE TABLE`, table-level CHECK in `CREATE TABLE`, and `ALTER TABLE … ADD CONSTRAINT … CHECK`. A tautological CHECK enforces nothing — almost always signals a copy-paste mistake or unfinished migration. New matcher `check_constraint_always_true`. New canonical TOML file `rules/quality.toml` carrying the `SC-CHK*` rule namespace.
- `scythe audit` now dispatches rules in the `Antipattern` category alongside `Security` and `Migration`, so non-security canonical rules surface in audit output. `--list-rules` groups SC-CHK* under a separate `[antipattern]` heading.
- `scythe lint` now runs the canonical SC-SEC*, SC-RLS*, SC-MIG*, and SC-CHK* audit packs alongside the existing schema-aware safety/codegen/naming rules and sqruff. Dialect gating: rules whose `dialects` list excludes the configured `[[sql]].engine` are silently skipped, so a `mysql` project does not see postgres-only `SC-MIG*` findings without explicit opt-in. No CLI flag is required — the rules ship in `default_registry()` and respect the same `[lint]` severity overrides as the rest of the rule set.
- Public `scythe-audit` pre-commit hook published via `.pre-commit-hooks.yaml`. Runs the canonical audit rule packs over staged `.sql` files with no `scythe.toml` required. Defaults to the postgres dialect; override per-hook via `args: [--dialect, mysql]`. The existing `scythe-lint` hook now also picks up audit rules whenever a `scythe.toml` is present. Documented in `docs/guide/pre-commit-hooks.md` and `docs/guide/audit.md`.
- Oracle bindings upgraded to sibyl 0.7. The codegen emitter (`crates/scythe-codegen/src/backends/rust_sibyl.rs`) was rewritten for sibyl 0.7's broken APIs: `sibyl::prelude` is gone (top-level re-exports used directly), `Varchar::as_str()` now returns `&str` instead of `Result<&str>`, and `Date::timestamp()` was removed (chrono::NaiveDateTime now built from the `date_and_time()` tuple). The integration test template selects `["tokio", "nonblocking"]`; without `nonblocking`, sibyl 0.7's `impl Debug for LOB` has every `fn fmt` body cfg-gated away and the lib fails to build. The Oracle manifest now maps `decimal` to `f64` because sibyl 0.7 has no `ToSql`/`FromSql` for `rust_decimal::Decimal` — flagged as a precision trade-off for follow-up.
- sqlx 0.8 → 0.9 in the Rust integration test crates (`rust-sqlx`, `rust-sqlx-mysql`, `rust-sqlx-mariadb`, `rust-sqlx-sqlite`, `rust-sqlx-redshift`). sqlx 0.9 tightened `raw_sql` and `query` to require `SqlSafeStr`; the integration test template now wraps runtime SQL strings with `sqlx::AssertSqlSafe`.

### Fixed

- Five `test_engines` codegen tests that were failing on `main` against the previous baseline are green. Three were neutral-type mappings falling through to the unknown-type literal fallback: MSSQL `DATETIMEOFFSET` → `datetime_tz` (was `"datetimeoffset"`), Redshift `SUPER` → `json` (was `"super"`), Oracle `NUMBER(p, s)` with a non-zero scale → `decimal` (was `int64` because `normalize_data_type` was ignoring the `Custom`-token scale parameter). Two were stale fixture expectations: Oracle `NUMBER(10)` correctly maps to `int64` (10 digits overflows int32), and Snowflake `INTEGER` correctly maps to `int32` (sqlparser parses it dialect-agnostically as `DataType::Integer(None)`; dialect-aware widening to int64 is tracked as a separate follow-up).

### Changed

- The four Postgres-specific audit rules (SC-SEC04 superuser-role, SC-SEC05 literal-password, SC-SEC10 security-definer-no-search-path, SC-SEC11 session-mutation) now declare `dialects = ["postgres"]` and no-op on non-PostgreSQL dialects instead of producing false positives. Behaviour is unchanged for the default PostgreSQL workflow.
- Pre-commit hook chain aligned with the polyrepo's shared `kreuzberg-dev/pre-commit-hooks v2.1.10` source. Nine individual hook repos collapsed into a single consolidated source for general file checks, markdown, Rust (fmt/clippy/sort/machete/deny), shell (shfmt/shellcheck), typos, and ai-rulez governance. `taplo-format` and `biome-format` stay as separate repos. `rustdoc-lint`, `markdownlint-rumdl-strict`, and `rust-max-lines` are listed in the config but commented out with TODOs — scythe's current codebase trips each one (~449 missing-doc errors, 35 long-line markdown files, 4 source files over 1,000 LOC); each is its own focused remediation. A new `_typos.toml` allowlists SQL aliases (`ba`), a singularize edge case (`statu`), the typos default dictionary's surprise prefix entries (`CHEC`→`CHECK`, `SELEC`→`SELECT`) that fire on plural SQL keywords, and excludes lockfiles where hex commit hashes routinely trip false matches.
- Sibyl-driven Oracle integration test now reads `schema.sql` instead of `schema_full.sql`. `schema_full.sql` contained PL/SQL `CREATE SEQUENCE … INCREMENT BY …` blocks that sqlparser cannot parse; the trimmed `schema.sql` carries only the `CREATE TABLE` DDL scythe actually needs for type inference. The test database setup still uses `schema_full.sql` separately.

## [0.8.0] - 2026-05-26

### Added

- Kotlin `extension_functions` backend option (opt-in, default off) for `kotlin-jdbc` and `kotlin-r2dbc`. When enabled, query functions are generated as idiomatic Kotlin extension functions on the connection receiver (`fun Connection.getUser(id: Int)` called as `connection.getUser(id)`) with expression bodies for value-returning queries. `kotlin-r2dbc` is reworked into a `suspend` extension on `io.r2dbc.spi.Connection`, moving the connection lifecycle to the caller. (#43)
- PHP `namespace` backend option for `php-pdo` and `php-amphp`. Any value emits `namespace <value>;`; an empty string omits the declaration. Default remains `App\Generated`, so existing output is unchanged. Enables PSR-4 framework integration (Laravel, Symfony, etc.). (#46)

### Fixed

- Schema parser no longer crashes on psql client meta-commands. `pg_dump 18+` and `dbmate` emit `\restrict` / `\unrestrict` lines that are not SQL; scythe now strips any line whose first non-whitespace character is `\` before parsing, so plain-format Postgres 18 dumps are consumed as-is. (#49)
- `python-psycopg3`, `python-asyncpg`, and `python-aiomysql` now emit `import uuid` and `from typing import Any` when their type mappings use `uuid.UUID` / `dict[str, Any]`. Generated modules previously raised `NameError` on import. (#48)

## [0.7.0] - 2026-05-20

### Added

- `scythe-core` now captures unknown `-- @<name> <value>` annotation lines as `CustomAnnotation { name, value, line }` triples on `Annotations.custom` and `AnalyzedQuery.custom`. Lets crate consumers layer their own annotation vocabularies (e.g. HTTP routing metadata) on top of scythe without coupling the SQL compiler to any one domain. Native annotations (`@name`, `@returns`, `@param`, `@nullable`, `@nonnull`, `@json`, `@optional`, `@group_by`, `@deprecated`) are unaffected — only previously-ignored unknowns are captured.
- `scythe-core` gained an optional `serde` feature that adds `Serialize` / `Deserialize` derives to the public IR types (`AnalyzedQuery`, `AnalyzedColumn`, `AnalyzedParam`, `EnumInfo`, `CompositeInfo`, `CompositeFieldInfo`, `GroupByConfig`, `QueryCommand`, `Annotations`, `ParamDoc`, `JsonMapping`, `CustomAnnotation`). Off by default.
- `Catalog::tables_iter()` accessor returning `(&String, &Table)` pairs, complementing the existing `tables()` (which returns names only).

### Fixed

- sqlparser 0.62 compatibility: handle multi-alias select items, object-name insert targets, and unsupported table-query insert targets so `cargo clippy --workspace -- -D warnings` is clean.

## [0.6.13] - 2026-05-10

### Fixed

- Generated Rust code is now rustfmt-clean — scythe invokes rustfmt on generated `.rs` files to ensure long function signatures are properly formatted across multiple lines, eliminating unnecessary diffs when downstream projects run `cargo fmt`

## [0.6.12] - 2026-05-07

### Fixed

- The 0.6.11 ON CONFLICT preprocessor scanned the raw SQL byte string, so text inside `--` line comments and `'…'` literals could trigger the predicate-stripping path and chew into the surrounding INSERT body. The scanner now runs against an ASCII-uppercase mask where comments + string literals are replaced with same-length spaces, so positions still line up but only structural SQL is matched.

## [0.6.11] - 2026-05-07

### Fixed

- PostgreSQL: accept `INSERT … ON CONFLICT (cols) WHERE … DO …` (the index-inference form for partial unique indexes). sqlparser-rs through 0.61 doesn't recognise the predicate, so scythe now strips it for the parser pass while keeping the original SQL for codegen and runtime, where Postgres validates and uses the predicate to pick the matching partial index. Mirrors the existing dialect-preprocess pattern used for Oracle and MSSQL.

## [0.6.10] - 2026-05-06

### Fixed

- Clippy warnings in `scythe-lint` style rules (`collapsible_match`) and `typescript-postgres` backend (`unnecessary_sort_by`)

### Changed

- Fixture data for pending engines (MSSQL, Oracle, Redshift, Snowflake) moved from `engines_pending/` to `testing_data/engines_pending/` — all fixtures now under one directory
- Updated pre-commit hooks: ai-rulez v4.1.6, rumdl v0.1.88, cargo-sort v2.1.4
- Bumped integration test dependencies: `rand` 0.8.5 → 0.8.6, `pgx/v5` 5.7.4 → 5.9.2, `gosnowflake` 1.10.1 → 1.13.3, `snowflake-sdk` 1.15.0 → 2.0.4, `snowflake-jdbc` 3.16.1 → 4.0.2

## [0.6.9] - 2026-04-15

### Fixed

- `scythe fmt` and `scythe lint` now auto-detect SQL dialect from `scythe.toml` when files are passed directly (e.g. by pre-commit hooks)
- PHP amphp: autoload vendor deps, use `query()` instead of `exec()`
- Ruby SQLite: handle `:exec` CreateUser/CreateOrder with post-insert fetch
- PHP SQLite: pass `status` param to `createUser`
- Oracle CI: install Instant Client SDK headers for ruby-oci8
- Snowflake CI: simplified to Python fakesnow only (no Docker emulator)
- Kotlin SQLite: Float literal types for total values
- Elixir jamdb Oracle: use `DBConnection.execute` and `schema_full.sql`
- Elixir Ecto: use Postgrex directly, fix `:one` empty result handling
- MariaDB C#: `GetValue().ToString()` for UUID columns (was `GetString()`)
- Oracle Go: EZ Connect format (`//host:port/service`) for godror

## [0.6.8] - 2026-04-15

### Added

- MSSQL integration tests across 10 backends (Rust tiberius, Python pyodbc, Go go-mssqldb, TypeScript mssql, Java JDBC, Kotlin JDBC, C# SqlClient, Elixir TDS, Ruby TinyTds, PHP PDO)
- Redshift integration tests across 13 backends (all PostgreSQL-compatible drivers with Redshift-specific manifests)
- Snowflake integration tests across 7 backends (Python, TypeScript, Go, Java, Kotlin, C#, PHP)
- MSSQL CI job with SQL Server 2022 Docker
- Redshift CI job using PostgreSQL container with PG-compatible schema
- Snowflake CI job with snowflake-emulator Docker + fakesnow for Python
- MSSQL `OUTPUT INSERTED` preprocessing: converts to `RETURNING` for parser, preserves original SQL in codegen
- Redshift `IDENTITY(N,N)` schema preprocessing: strips before parsing
- Snowflake type mappings: `TIMESTAMP_NTZ`, `TIMESTAMP_TZ`, `TIMESTAMP_LTZ`, `VARIANT`
- 89 total integration test backends (up from 69)

### Fixed

- CI: `libaio1` → `libaio1t64` for Ubuntu 24.04 (Oracle job)
- CI: SQLite `create_if_missing(true)` + `touch` step
- CI: removed committed macOS `.bundle/config`
- Go codegen: `@pN` placeholder rewriting for MSSQL
- Rust tiberius codegen: `Compat<TcpStream>` type, `&dyn ToSql` param binding, string `FromSql` handling
- Ruby TinyTds codegen: type-aware param escaping (integers/booleans not escaped)
- TypeScript mssql codegen: explicit `sql.*` type bindings for params
- Template fixes for Redshift (no enums, `schema_pg_compat.sql`, status as string)
- Elixir: `elixirc_paths` includes `generated/` for all backends
- TypeScript: `String()` coercion for decimal total comparisons

### Unverified / Skipped in CI

The following backends have codegen support but are **not tested in CI** due to driver/infra limitations:

**MSSQL:**

- `elixir-tds` — Elixir `tds` library parameter type encoding fails ([#28](https://github.com/Goldziher/scythe/issues/28))

**Oracle:**

- `elixir-jamdb` — `DBConnection.ConnectionPool` dispatch error with `jamdb_oracle`
- `ruby-oci8` — native gem requires Oracle Instant Client SDK headers not available in CI

**SQLite:**

- `php-pdo-sqlite` — generated `createUser` param count mismatch with test template

**Snowflake** ([#27](https://github.com/Goldziher/scythe/issues/27)):

- `go-gosnowflake` — no free Snowflake emulator with full Go driver support
- `typescript-snowflake` — emulator doesn't support TS SDK protocol
- `java-jdbc-snowflake` — emulator doesn't support JDBC protocol
- `kotlin-jdbc-snowflake` — emulator doesn't support JDBC protocol
- `csharp-snowflake` — emulator doesn't support .NET driver protocol
- `php-pdo-snowflake` — emulator doesn't support PDO protocol

Only `python-snowflake` is tested via [fakesnow](https://github.com/tekumara/fakesnow) (in-process DuckDB).

## [0.6.7] - 2026-04-12

### Added

- Oracle integration tests across 9 backends (Python oracledb, TypeScript oracledb, Go godror, Java JDBC, Kotlin JDBC, C# Oracle, Elixir jamdb, Ruby oci8, Rust sibyl)
- Oracle CI job with Oracle XE 21 and Instant Client
- Oracle SQL support: `:N` placeholder preprocessing, `RETURNING ... INTO` output bind codegen
- Oracle `orders.sql` queries with `RETURNING INTO` support
- `structs_only` option for Rust sqlx backend (skips `sqlx::query!()` macros that require compile-time DB)

### Changed

- Java codegen: emit `package generated;` and `public class Queries { ... }` wrapper — eliminates hand-written wrapper files
- Kotlin codegen: emit `package generated` header
- Java output path: `src/main/java/generated/Queries.java`; Kotlin: `src/main/kotlin/generated/queries.kt`
- Rust sqlx integration tests output to `src/queries.rs` with `structs_only` mode
- Oracle dialect uses `OracleDialect` from sqlparser (was `GenericDialect`)

### Fixed

- Go database-sql MySQL: fixed connection failure when `MYSQL_URL` uses `mysql://` URL format
- Ruby mysql2 MySQL: regenerated code to use `stmt.affected_rows` (fixes incorrect `DELETE` row counts)
- Java/Kotlin JDBC: enum columns read via `valueOf(toUpperCase())` instead of broken `getObject()`
- Java/Kotlin JDBC: PostgreSQL enum params use `setObject(Types.OTHER)`, others use `setString(getValue())`
- Java/Kotlin JDBC MariaDB: `RETURNING` queries use `execute()` + `getResultSet()` (MySQL Connector/J doesn't support `executeQuery()` for DML RETURNING)
- Rust sqlx MariaDB: UUID columns cast to `CHAR` in all queries (sqlx can't decode MariaDB BINARY UUID)
- Rust sqlx MariaDB/MySQL: use `last_insert_id()` from result instead of `LAST_INSERT_ID()` SQL function (pool connection mismatch)
- Rust sqlx: `raw_sql()` for multi-statement schema loading (PG and SQLite)
- MariaDB manifests: UUID mapped to `String` for Rust sqlx, Java JDBC, Kotlin JDBC (drivers return String, not UUID object)
- Java imports: `java.time.*` wildcard for all temporal types

## [0.6.6] - 2026-04-12

### Added

- MariaDB integration tests across all 11 supported backends (Rust sqlx, Python aiomysql, TypeScript mysql2, Go database/sql, Java JDBC, Kotlin JDBC, C# MySqlConnector, Elixir MyXQL, Ruby mysql2, Ruby trilogy, PHP PDO)
- MariaDB CI job running all 11 backends against MariaDB 11
- MariaDB `orders.sql` queries with `INSERT...RETURNING` support

## [0.6.5] - 2026-04-12

### Added

- Java JDBC and Kotlin JDBC: Oracle backend support

### Fixed

- tokio-postgres: enums now implement `FromSql` and `ToSql` traits natively, enabling direct use as query parameters and row fields without manual string conversion
- Ruby mysql2: `affected_rows` now called on the statement instead of the client, fixing incorrect return values for exec queries

## [0.6.4] - 2026-04-10

### Added

- Integration tests now run all generated code against real databases (PostgreSQL, MySQL, SQLite) across all 39 backends and 10 languages
- CI split into 3 parallel jobs (PostgreSQL, MySQL, SQLite) covering all backends
- New MySQL/SQLite SQL queries: GetUserOrders, CountUsersByStatus, GetUserWithTags

### Fixed

- tokio-postgres: enum parameters now use `::text::enum_name` casts for proper PostgreSQL enum handling
- tokio-postgres: enum columns in SELECT/RETURNING use `::text` cast for correct deserialization
- sqlx: RETURNING clauses now include enum type annotations (`"status: UserStatus"`)
- sqlx: aggregate functions (COUNT, SUM) get non-null override annotations (`"column_name!"`)
- C# Npgsql: enum extension methods moved to top-level static classes (fixes CS1109)
- C# Microsoft.Data.Sqlite: fixed type mappings (int32->long, float32->double for SQLite)
- Elixir exqlite: updated to Exqlite 0.36 prepare/bind/step API
- Elixir myxql/exqlite/ecto: generated code now properly wrapped in `defmodule`
- Python aiomysql: `?` placeholders correctly rewritten to `%s`
- Go pgx: added missing `time` and `decimal` imports in generated code
- Ruby trilogy: parameterized queries use string interpolation (trilogy lacks prepared statement support)
- TypeScript pg-zod: enum columns use correct Zod schema references

## [0.6.3] - 2026-04-10

### Added

- `fmt` and `lint` commands now auto-detect the SQL dialect from the config `engine` field (CLI `--dialect` flag still takes precedence)

### Fixed

- Sqruff rule `LT01` excluded by default — it incorrectly splits compound operators (`>=`, `<=`, `<@`)
- Compound operators inside CHECK constraints no longer get split by the formatter (e.g., `>=` becoming `> =`)

## [0.6.2] - 2026-04-10

### Changed

- tokio-postgres: `from_row` is now infallible (returns `Self` instead of `Result`) matching tokio-postgres `row.get()` conventions
- tokio-postgres: all query functions uniformly return `Result<T, tokio_postgres::Error>` instead of mixed error types
- tokio-postgres: extracted `ERROR_TYPE` constant to reduce string duplication in signatures

### Fixed

- `:opt` command now correctly generates row structs (was missing from struct generation match)

## [0.6.1] - 2026-04-10

### Added

- `:opt` query command across all backends — returns optional/nullable single row (distinct from `:one` which expects exactly one row)
- Serde and custom derive support for tokio-postgres backend via `serde` and `derive` options
- `apply_options()` method on tokio-postgres backend for runtime configuration
- `is_column_nullable()` helper on analyzer scope for nullable column lookups
- `collect_param_from_expr_with_type_nullable()` for nullable-aware parameter collection
- `version:sync` task in Taskfile for updating all crate versions at once

### Changed

- tokio-postgres: `client` parameter now accepts `&(impl GenericClient + Sync)` instead of concrete `&Client`
- tokio-postgres: batch functions no longer wrap operations in implicit transactions

### Fixed

- INSERT parameter analysis now propagates column nullability to parameters
- Changelog retroactively aligned with Cargo.toml version history (0.1.0–0.6.0)

## [0.6.0] - 2026-04-08

### Added

- Microsoft SQL Server engine (6 backends: tiberius, pyodbc, mssql, sqlclient, tiny_tds, tds)
- Oracle Database engine (6 backends: sibyl, oracledb, godror, oracle, oci8, jamdb)
- MariaDB engine with native UUID support, RETURNING clause, and dedicated manifests
- Amazon Redshift engine (PostgreSQL-based with SUPER type support)
- Snowflake engine with VARIANT/OBJECT/ARRAY types
- 17 new database backends and 51 type mapping manifests
- Pre-commit/prek hooks for scythe users

### Changed

- Flattened docs structure for better organization
- Expanded to 10 total databases with 70+ backend drivers across 10 languages

### Fixed

- Extracted shared `rewrite_pg_placeholders` function (eliminated 26+ duplicated functions)
- Extracted shared `load_or_default_manifest` function (eliminated 49 duplicated code blocks)
- CockroachDB documentation TOML snippet duplicate key issue
- Python DuckDB missing datetime import
- TypeScript DuckDB import type issue
- Go godror PascalCase conversion issue
- Go unconditional imports problem
- SQLx hardcoded PgPool issue
- Tiberius unwrap error handling
- Kotlin wasNull null handling
- Ruby batch operation fix
- Sibyl error swallowing issue
- Go `interface{}` updated to `any` keyword

## [0.5.0] - 2026-04-08

### Added

- CockroachDB engine support
- DuckDB engine support
- `:grouped` operation support
- Kotlin Exposed backend
- R2DBC backend support
- Homebrew bottles for distribution
- Integration test generator for all 39 backend test suites

## [0.4.0] - 2026-04-08

### Added

- Real `:batch` operations across all backends
- PHP AMPHP backend
- Custom type overrides feature
- `@optional` annotation support
- Elixir Ecto backend
- Ruby Trilogy backend
- Pydantic/msgspec row types for Python
- Zod v4 schemas for TypeScript
- GenOptions infrastructure for per-backend configuration

### Changed

- Extended Quick Start documentation with all 10 languages

## [0.3.0] - 2026-04-07

### Added

- Snippet-runner tool for validating documentation code snippets across 13 languages
- PHP namespace support and Generator for `:many` queries
- C# SQLite async API
- Ruby module `Queries` encapsulation across all 3 backends

### Changed

- C# all backends: Enum.TryParse with descriptive InvalidOperationException
- Python aiosqlite: Decimal maps to `decimal.Decimal` instead of float
- Go database-sql MySQL: Decimal maps to float64
- Ruby: SCREAMING_SNAKE_CASE enum variants
- PHP: Final class `Queries` wrapper

### Fixed

- 8 backend-specific fixes across PHP, Ruby, C#, Rust, Python, and Go

## [0.2.0] - 2026-04-07

### Added

- Engine-aware backend architecture — `get_backend(name, engine)` loads engine-specific manifests
- 12 new language backends for MySQL and SQLite: go-database-sql, python-aiomysql, python-aiosqlite, typescript-mysql2, typescript-better-sqlite3, ruby-mysql2, ruby-sqlite3, csharp-mysqlconnector, csharp-microsoft-sqlite, elixir-myxql, elixir-exqlite
- Multi-backend CLI config via `[[sql.gen]]` array syntax in scythe.toml
- Full MySQL support across all 10 languages (Rust, Go, Python, TypeScript, Java, Kotlin, C#, Elixir, Ruby, PHP)
- Full SQLite support across all 10 languages
- 33 real integration tests against PostgreSQL, MySQL, and SQLite
- `supported_engines()` method on CodegenBackend trait for engine validation
- `manifest()` method on CodegenBackend trait for direct manifest access
- `file_footer()` method on CodegenBackend trait for class wrappers (C#)
- Engine-specific manifest files for multi-DB backends (java-jdbc, kotlin-jdbc, php-pdo, rust-sqlx)
- Docker Compose setup for integration testing (PostgreSQL + MySQL)

### Changed

- `get_backend()` now requires engine parameter for database-aware code generation
- Backend constructors accept engine parameter and load appropriate manifests
- PG-only backends reject non-PostgreSQL engines with clear error messages

### Fixed

- Python codegen: multiline SQL now uses triple-quoted strings
- Python codegen: added missing `import decimal` to file headers
- TypeScript pg codegen: multiline SQL now uses backtick template literals
- C# codegen: generated code now wrapped in `public static class Queries { }`
- C# codegen: enum parameters use `.ToString().ToLower()` with `::enum_type` SQL cast
- C# codegen: enum columns deserialized via `Enum.Parse<T>(reader.GetString(i), true)`
- PHP codegen: MySQL `?` placeholders use positional arrays instead of named params
- PHP codegen: enum params use `->value`, enum columns use `::from()`, DateTimeImmutable for timestamps
- Go codegen: added missing `time` and `decimal` imports to file header
- Java codegen: added import statements to file header
- Ruby mysql2 codegen: `affected_rows` called on statement instead of client

## [0.1.0] - 2026-04-06

### Added

- SQL-to-code generation for 13 language backends:
  - Rust (sqlx, tokio-postgres)
  - Python (psycopg3, asyncpg)
  - TypeScript (postgres.js, pg)
  - Go (pgx v5)
  - Java (JDBC with records)
  - Kotlin (JDBC with data classes)
  - C# (Npgsql with records)
  - Elixir (Postgrex with defstruct)
  - Ruby (pg gem with Data.define)
  - PHP (PDO with readonly classes)
- Database dialect support: PostgreSQL, MySQL, SQLite
- SQL annotation system (@name, @returns, @param, @nullable, @nonnull, @json, @deprecated)
- Smart type inference with nullability propagation (JOIN, COALESCE, aggregates, CASE)
- Language-neutral type vocabulary with per-backend type mapping via manifest.toml
- 93 SQL lint rules (22 scythe-specific + 71 via sqruff integration)
- SQL formatting via sqruff integration
- CLI commands: generate, check, lint, fmt, migrate
- sqlc migration tool (convert sqlc.yaml to scythe.toml, migrate query annotations)
- 275 JSON test fixtures with auto-generated test code
- Real language tool validation (ruff, biome, gofmt, ktlint, ruby -c, php -l)
- Template-based backend architecture (manifest.toml + MiniJinja templates)
- Trait-based CodegenBackend for extensible language support
