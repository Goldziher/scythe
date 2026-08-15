# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security

- **Every PHP integration harness created an order and never checked it was the one returned.**
  The same defect fixed for all 13 Python harnesses in 0.15.0 was left live in all 9 PHP ones:
  `test_create_order` returns the new row's id and `test_get_orders_by_user` ignored it, asserting
  only the first result's `notes`, so a query returning someone else's order still passed.
  `test_get_orders_by_user` now takes the created `order_id` and asserts it is among the returned
  rows. (#112)

### Fixed

- **The `python-psycopg3-msgspec` harness never checked its rows were msgspec structs.** The
  project exists to prove the `row_type = "msgspec"` codegen option works, and its Pydantic twin
  carries seven assertions — a dedicated row-type test plus five `isinstance` checks — while the
  msgspec harness had none; `import msgspec` was the only trace of it, unused. It now mirrors the
  Pydantic assertions. Three unconditional imports (`asyncio`, `Decimal`, `msgspec`) that were
  unused on some engine branches are now emitted only where used, so the generated Python harnesses
  are `F401`- and `I001`-clean. (#112)

- **Every codegen assertion in the fixture-generated test suite was skipped when codegen errored.**
  One line in the generator wrapped each backend loop in `if let Ok(generated) = …`, producing 273
  skip-guards across 13 files that between them discarded the result of **4993**
  `generate_with_backend` calls. Backend *construction* failure already panicked; generation failure
  one line later did not — and `generate_generated_code_assertions`, added in 0.15.0 specifically to
  stop assertions being dropped, was emitted *inside* that guard, so the fix for dropped assertions
  was itself dropped. A codegen error now fails the test naming the backend, engine, fixture and
  error. A fixture may declare an expected failure via `expected.codegen_errors`, which requires a
  written reason and fails in both directions: an undeclared failure fails, and a declared failure
  that now succeeds fails as stale. No fixture currently declares one — measured across all 4993
  combinations, none fail. (#222)

- **A set-returning function in the select list, and a multi-field `ROW(...)`, passed analysis and
  then failed every backend with `INTERNAL_ERROR: unknown neutral type: unknown`.**
  `SELECT jsonb_each(data) FROM documents` and `SELECT array_agg(ROW(o.id, o.total)) FROM orders o`
  both reported an internal error — "file a bug against scythe" — for input scythe had diagnosed
  perfectly well. PostgreSQL's anonymous `record` and a bare multi-field row genuinely have no
  neutral type, but that is a fact to report, not an internal fault. Both now fail at analyze time
  with `UNRESOLVED_TYPE`, naming the column and the construct; the set-returning-function message
  points at the `FROM`-clause form (`FROM documents, jsonb_each(data) AS kv`), which already
  resolves to real `key` and `value` columns. The same treatment covers `json_each_text`, the
  `json_populate_record` family, `unnest` over a non-array, and nine other expression shapes that
  previously reached codegen as a bare `"unknown"`. (#223)

- **An unresolved marker wrapped in a container leaked its internal spelling to the user.**
  `SELECT array_agg(bogus_fn(id)) FROM t` reported
  `INTERNAL_ERROR: unknown neutral type: __unknown_func__:bogus_fn` — scythe's own internal marker,
  verbatim. The markers that stand for "ambiguous column", "unknown column" and "unknown function"
  were matched only at the start of a neutral type, so `array_agg` wrapping one as
  `array<__unknown_func__:…>` slipped past every check. This is the #173 failure mode the marker
  family's own doc comment warns about, still live for the container case. It now reports
  `UNKNOWN_FUNCTION: function "bogus_fn" does not exist`.

- **Two CLI integration tests gated their generated output on its byte count.**
  `test_generate_pagila_writes_file`'s entire body, after checking the file existed, was
  `content.len() > 500` — pagila generates 7016 bytes, so the check permitted losing 93% of it, and
  `test_generate_writes_file`'s `> 100` was no better. Both now assert that every query in the
  fixture produced a named function, and that the file defines exactly that many and no more; the
  count is what catches two queries collapsing onto one name, which a presence check cannot see.
  (#161)

- **A composite-typed query parameter was bound to postgres.js as a whole object, so
  `typescript-postgres` output did not type-check** (`TS2345: 'TortureAddress | null' is not
  assignable to 'ParameterOrFragment<never>'`). The codegen that renders `ROW(a, b)::type_name` for
  a bound composite was already correct but never ran: it looks the composite up in
  `analyzed.composites`, and the analyzer's composite worklist seeded itself from a query's columns
  and nested-struct fields but never from its params. A composite bound only as a parameter — an
  `INSERT` whose composite column never appears in `RETURNING` — therefore never reached that list
  at all, and the emitter took its silent whole-object fallback. The worklist now chains params the
  way the enum scan beside it always did. This was the last entry in
  `scripts/torture-expected-failures.txt`, which is now empty. (#225)

- **A `UNION` whose `NULL`-projecting arm came first failed type resolution instead of widening.**
  `SELECT id AS tag FROM accounts UNION SELECT NULL AS tag FROM users` compiled; swapping the arms
  produced `INTERNAL_ERROR: type resolution failed for column 'tag': unknown neutral type: unknown`.
  `widen_union_arm_type`'s non-nested fallthrough called `widen_type` directly, and `widen_type`
  returned its left argument for any pair its numeric ladder does not handle — so an `unknown` arm
  on the left won over the other arm's real type. `widen_type` now absorbs `unknown` from either
  position, and the call site routes through `widen_neutral_type`, the helper whose own doc comment
  names it the single rule every widening call site must use (#121) and which every other call site
  already used. `UNION` is commutative, so both spellings now agree. Reported and fixed by
  @snowyukitty in #227. (#224)

### Added

- **`javascript-node-sqlite`: a JSDoc emit mode for `typescript-node-sqlite`.** The fifth
  `javascript-*` backend (alongside `javascript-postgres`, `javascript-pg`, `javascript-mysql2`,
  `javascript-better-sqlite3`): plain, JSDoc-annotated `.js` output for Node's built-in
  `node:sqlite` module, checked against real `node --check` and `tsc --checkJs --strict` in CI.
  `node:sqlite` is synchronous, like `better-sqlite3`, so this mirrors
  `javascript-better-sqlite3`'s emit shape rather than the `async` pg/postgres.js/mysql2 one, except
  for `:batch`: `DatabaseSync` has no `.transaction()` helper, so the generated code wraps explicit
  `BEGIN`/`COMMIT`/`ROLLBACK` statements, matching `typescript-node-sqlite`'s own TS-mode `:batch`
  shape.

- **The `javascript-*` backends' `:many` output is now type-checked by real `tsc`.** The JS-mode
  tool-validation fixture built only a `:one` and a `:grouped` query, so the one command whose JSDoc
  cast is not a plain one-step assertion was pinned by hand-written string matching alone, on all
  five backends. It now builds a `:many` query too. (#93)

## [0.15.0] - 2026-08-15

This release is mostly about checks that could not fail. A validator whose only callers were its own
tests, an allowlist nobody reconciled, a CI step whose assertion was vacuous, a fixture that asserted
only that *something* was generated — each one reported success while measuring nothing, and several
had done so since the feature they guarded shipped. Auditing them found real defects underneath:
generated Rust whose bytes depended on whether `rustfmt` happened to be on `PATH`, two queries whose
names collapsed onto one function, a nested aggregate quietly degraded to an opaque string, and a
`ruby-pg` signature promising a `Hash` for a value that was the driver's raw wire text.

A second shape recurred often enough to name: the test that pins the bug. Several tests asserted the
*defective* output verbatim, so they failed when someone fixed the thing they were named after. Those
are inverted here, each with a doc comment stating what it now guards.

Nullability, JVM enum round-trips, `?` placeholder counting and Oracle's LOB reads were all measured
against live PostgreSQL, MySQL, MariaDB and Oracle rather than against scythe's own model. Four
backends that had never executed anywhere — `kotlin-exposed` among them — now run in CI, and running
them found seven defects no string-matching test could reach.

**Upgrading**: three changes can turn a config that used to be accepted into an error. `[lint.sqruff]`
is now actually read, so a table that was previously inert may now fail the run; keys in a `[[sql]]`
block that scythe does not define are now rejected instead of ignored; and Ruby `.rbs` output changes,
so committed signatures need regenerating. `scythe-codegen`'s public `generate_from_catalog` stub is
also removed (#132) — a breaking change for any direct caller, though it had none in this repository.
Two lint-crate suppression and audit APIs also changed shape (see **Fixed**): `SuppressionSet` is now
keyed by statement index instead of source line, and `LintRule` gained `cwe()` / `is_applicable_to()`
methods with safe defaults. `scythe-lint` also drops four `sqruff_adapter` free functions in favour of
building a `SqruffLinter` once (see **Removed**). `scythe-core`'s public `CustomAnnotation` struct
gained a `suggested_keyword: Option<String>` field (see **Fixed**, #152) — a breaking change for any
direct caller that builds one by struct literal rather than through the parser. Details below.

### Security

- **`SC-RLS02` (`policy-always-permissive`) reported a deny-all RLS policy as granting
  unconditional access.** `WITH CHECK (NULL)` / `USING (NULL)` reject every row — NULL is not
  TRUE — but the rule's tautology check folded `NULL` in alongside `true` and `1=1`, so the most
  restrictive policy possible was flagged at `error` severity with remediation advice ("replace
  the tautology with an actual predicate") that would have *loosened* security in response to a
  security finding. `NULL` is no longer treated as a tautology by this rule; `SC-CHK01`
  (`check-constraint-always-true`), where `NULL` genuinely does satisfy a CHECK constraint, is
  unaffected. (#139)
- **Every Python integration harness created an order and never checked it was the one returned.**
  `test_create_order` returns the new row's id, but `test_get_orders_by_user` ignored it and only
  asserted the first result's `notes`, so a query that returned someone else's order (or the wrong
  row) would still pass. `test_get_orders_by_user` now takes the created `order_id` and asserts it
  is present in the returned rows, in all 13 Python harnesses. (#112)

- **`java.java.jinja` and `kotlin.kt.jinja`'s non-postgresql engine branches were missing tests for
  queries their own fixtures already defined.** GH #195/#196's parity gate (`10066723`) made the
  drift visible via `test-parity-exemptions.txt` but left the 44 "never wired up" gaps open;
  `UpdateUserEmail` and `SearchUsers` are now called from every engine branch in both templates,
  `GetOrderTotal` from every branch that didn't already have it (duckdb, mssql, redshift,
  snowflake, sqlite), and `ListActiveUsers` from redshift's. Redshift's `SearchUsers` and
  `ListActiveUsers` queries filter by `status`, not a name `LIKE` pattern like every other engine —
  its ported tests call them with a status value rather than `"%Alice%"`, matching what
  `queries/users.sql` actually defines for that engine. The 44 closed exemption lines are deleted;
  the 48 remaining entries are all structural (a `UserStatus` enum parameter or the nullable
  composite-column read from board #197 with no per-engine equivalent) and are unchanged.

### Added

- **`kotlin-exposed` has a running integration project, and running it found seven defects.** The
  backend had shipped since 0.6.0 with nothing ever executing its output, and none of the seven was
  reachable by a string-matching test: the generated file declared no `package generated`, so any
  caller importing it failed outright; an enum parameter was bound as the Kotlin enum object rather
  than its SQL spelling; those parameters then needed an explicit `::<enum type>` cast, because
  Exposed sends a typed `character varying` that PostgreSQL will not coerce to a user enum;
  `:exec_rows` and `:exec_result` read a row count off `Transaction.exec`, which returns `Unit`, so
  they never compiled; a `RETURNING` query ran as an `INSERT` and the driver raised "A result was
  returned when none was expected", now fixed with an explicit `StatementType.SELECT`; the bind list
  was an unannotated `listOf(...)` whose type inference collapsed on a heterogeneous parameter set;
  and `UUIDColumnType` was emitted but never imported. The project runs 14 assertions in CI,
  including the composite-escaping and nullable-enum reads. (#213, #214)

- **`java-r2dbc` and `kotlin-r2dbc` have running integration projects on PostgreSQL.** Both backends
  shipped with nothing executing their output, and running them found defects no string-matching
  test could see: an enum parameter was bound as the Java/Kotlin enum object, which
  r2dbc-postgresql cannot encode, and once bound as its SQL spelling the server rejected the
  untyped `character varying` against a `user_status` column. Enum placeholders now carry an
  explicit `::<enum type>` cast on PostgreSQL, so the generated code needs no `EnumCodec`
  registration from the caller. The MySQL, MariaDB and SQLite pairs stay uncovered, each with a
  measured reason recorded in `tools/integration-test-generator/coverage-exemptions.txt`.

- **`php-amphp` on MySQL and `typescript-kysely` on Redshift now have integration projects that
  actually run in CI.** Both manifests shipped with nothing exercising them. `php-amphp-mysql`
  immediately found two real defects — the harness's `MysqlConfig::fromArray()` does not exist, and
  the generated pool type made `LAST_INSERT_ID()` unreliable (see **Fixed**) — which is the whole
  point of the exemption list these two came off. `typescript-kysely-redshift` gates the queries
  Redshift's fixture does not define and reads `status` as a varchar rather than an enum, matching
  what the `pg` and `postgres` drivers already did for that engine.

- **`rust-tokio-postgres` can read and write range columns.** The manifest previously declared no
  `range` mapping at all, because `postgres-types` ships no `Range<T>` with a `FromSql`/`ToSql`
  impl the way `sqlx-postgres` does, and the mapping it used to carry (`String`) could not decode:
  `String`'s `accepts()` matches no range OID, so `row.get` panicked before `from_sql` ran. The
  backend now emits a hand-rolled `PgRange<T>` built on `postgres_protocol::types::range_from_sql`
  / `range_to_sql` — the same wire-format primitives `postgres-types` uses internally for arrays —
  gated on a generated fragment actually naming `PgRange<`, so a file with no range column does not
  carry it. `Empty` is a distinct variant from a fully-unbounded range rather than collapsed into
  it, because the two are different values on the wire and collapsing them would make an empty
  range decode as if it contained everything. Verified against live PostgreSQL across bounded,
  empty, unbounded and both binding directions; note that no schema in this repository has a range
  column yet, so no CI job compiles the emitted wrapper. (unfiled)

- **Integration coverage for nullable enum and nullable composite columns.** No integration project
  had ever selected a composite column, so the entire runtime read path was unexercised while
  codegen compiled green. The PostgreSQL schema gains a `user_address` composite and two nullable
  columns, and a `GetUserProfile` query asserts both a present value and a SQL NULL — the shape
  that catches a reader which decodes NULL as a zero-valued variant or an all-default struct.

  Running it revealed that composite decoding is implemented in only four of the fifteen PostgreSQL
  backends: `rust-sqlx` and `rust-tokio-postgres`, which get it from their drivers' derive macros,
  and `java-jdbc` and `kotlin-jdbc`, which parse the composite text form. In the other eleven the
  generated row type declares the composite struct while the driver's raw value is assigned straight
  through, so the annotation is wrong at runtime — `php-pdo`, `php-amphp` and `csharp-npgsql` throw,
  and `python-psycopg3`, `python-asyncpg`, the `typescript-pg` family, `ruby-pg`, `elixir-postgrex`,
  `elixir-ecto` and `go-pgx` return a raw string, a driver record, or `undefined` with no error at
  all. The new assertions are therefore scoped to the four backends that work, with each excluded
  language's template carrying a note to restore them once that backend learns to parse a composite,
  so the gap is explicit rather than a green suite that proves nothing. (unfiled)

- **`scythe inspect` now has a real MySQL/MariaDB driver.** Live inspection was PostgreSQL-only;
  every other engine fell through to a stub that reported itself as `mysql` regardless of what the
  user asked for. `MySqlDriver` (backed by `mysql_async`) ships four checks driven by its own
  `mysql/checks.toml`, merged into the canonical registry alongside PostgreSQL's: `SC-INS-MY01`
  (no primary key), `SC-INS-MY02` (duplicate index), `SC-INS-MY03` (`AUTO_INCREMENT` past 70% of its
  type range), `SC-INS-MY04` (`MEMORY` storage engine). The two check sets are deliberately not
  symmetric — PostgreSQL's row-level-security, extension and `SECURITY DEFINER` search-path checks
  have no MySQL equivalent and are not approximated — and `verify_queries` stays PostgreSQL-only
  because it depends on the extended-query protocol's describe response. SQLite, MSSQL, Oracle,
  Snowflake and Redshift still get `UnsupportedDriver`, which names the engine the user actually
  asked for and refuses rather than returning an empty finding set. (#131, partial)

- **`scythe generate --validate-output`** runs the generated code through the real compiler or linter
  for its language and reports, per target, whether it was `VALIDATED`, `SKIPPED`, or `FAILED`.
  `validate_generated_code` previously had no production caller at all — every call site outside
  `validation.rs` was a test — so `generate` never checked its own output. Off by default because it
  shells out to toolchains that may not be installed. A run where the validator found no tool to
  invoke is reported as `SKIPPED`, never as success: reporting it as validated would recreate the
  unfalsifiable gate the flag exists to close. A `FAILED` target exits 2, matching the exit-code
  contract `check`/`lint`/`fmt --check` follow, where exit 1 stays reserved for operational failure.
  (unfiled)

- **DuckDB integration coverage.** `python-duckdb`, `typescript-duckdb`, `java-jdbc-duckdb` and
  `kotlin-jdbc-duckdb` now run in a new `integration-duckdb` CI job, against the schema and query
  set added earlier in this release. DuckDB is embedded, so the job needs no service container.
  `go-database-sql-duckdb` exists and its harness is written, but stays exempt: `go-duckdb` cannot
  bind a nil pointer, and the backend emits `*T` for a nullable parameter, so any NULL argument
  fails at runtime — measured against the driver, tracked as board #228. (#126)

- **`scythe-inspect` can read a SQLite or MySQL catalog.** A new `SchemaCatalogDriver` trait gives
  catalog reading the engine seam it never had — `fetch_live_schema` was a bare function hardcoded to
  `tokio_postgres::Client`. `SqliteCatalogSource` reads `sqlite_master` plus `PRAGMA table_info` and
  needs no server, so it is tested in-process; `MySqlCatalogSource` reads `information_schema`, with
  live tests gated the way the PostgreSQL ones already are. `ColumnDescription` gained `primary_key`.
  At the time this landed, the `SC-INS` health checks were still PostgreSQL-only hand-written
  `pg_catalog` SQL and the CLI was not wired to either source, so `scythe inspect`'s user-facing
  behaviour was unchanged — MySQL/MariaDB got a real `SC-INS` driver and honest CLI dispatch
  separately (see the "`scythe inspect` now has a real MySQL/MariaDB driver" entry above). What this
  entry's `SchemaCatalogDriver` sources still are not wired into is schema drift: nothing yet feeds
  `SqliteCatalogSource` or `MySqlCatalogSource` output into `diff_schemas` the way `fetch_live_schema`
  is for PostgreSQL, so drift detection stays PostgreSQL-only.
- **Generated Python and PHP are type-checked in CI.** A new `validate-generated-types` job installs
  each project's real driver, then runs `pyrefly check -p strict` over five Python backends and
  PHPStan over both PHP ones. Every step was proven able to fail by injecting a defect first. The
  `strict` preset is load-bearing — pyrefly's default `basic` preset misses a wrong return-type
  annotation entirely — and the job already catches one real pre-existing bug. Ruby and Elixir were
  investigated and deliberately left out rather than given a step that cannot fail: no Ruby driver
  has signatures in `gem_rbs_collection` (and `rbs validate` returns 0 even for a nonexistent class),
  and Dialyzer needs `dialyxir`'s translation layer, which no integration project depends on.

### Fixed

- **The java/kotlin engine-test-parity gate never measured five of its twelve branches, and one
  measured branch silently overwrote another's count.** `branch_test_names` only recognised a
  top-level `if`/`elif engine == "..."` line, so `driver == "r2dbc"`-conditioned branches and
  `backend == "kotlin-exposed"` were invisible to it entirely — 21 java and 35 kotlin test
  functions sat outside every window it built and were excluded from every comparison without a
  trace, leaving `integration_tests/java-r2dbc`, `kotlin-r2dbc`, and `kotlin-exposed` with zero
  parity coverage. Separately, a nested column-0 `{% if engine == "mariadb" %}` inside
  `kotlin.kt.jinja`'s r2dbc branch was mistaken for a real top-level branch, and its measured test
  set was overwritten by the real `mariadb` branch's via a plain `BTreeMap::insert` with no
  warning. Branch discovery now derives a key from every quoted literal a top-level condition
  compares against (`r2dbc-postgresql`, `r2dbc-mysql-mariadb`, `kotlin-exposed`, alongside the
  existing per-engine keys), a duplicate derived key is now a hard `panic!` naming both branch
  starts instead of a silent overwrite, and a new assertion fails if any test function falls
  outside every measured branch range. The newly measured `kotlin-exposed` gap was closed by
  renaming a test to match its postgresql counterpart; the remaining genuine gaps (all in the two
  `r2dbc` branches, one of which — mysql/mariadb — has no generated project to run it) are recorded
  in `test-parity-exemptions.txt` with reasons specific to why porting isn't safe or possible yet,
  taking that file from 48 entries to 60. (#195)

- **`ruby-pg` declared a `json`/`jsonb` column `Hash` in its `.rbs` but never decoded it.**
  `ruby-pg.toml` maps `json = "Hash"`, but `ruby_coercion` had no arm for it, so the generated
  `.rb` code read the bare `row["col"]` — the `pg` gem does no client-side JSON decoding, so
  that value was the raw wire-format `String`, contradicting the `Hash[String, untyped]` its
  own `.rbs` signature promised. This is the `json` sibling of #198's `decimal` bug, left open
  when that fix only covered `BigDecimal`. `ruby_coercion` now wraps a `json`/`json_array`
  column's value in `JSON.parse(...)`, gated behind a conditional `require "json"` the same way
  `.to_d` gates `require "bigdecimal/util"`. `json_array` (the `json_agg` array shape) is a new
  manifest scalar mapped to `Array`, so a degraded nested aggregate keeps declaring an `Array`
  instead of falsely claiming `Hash`. (#147)

- **`php-amphp` typed its handle as `SqlConnectionPool`, which made MySQL's generated
  `LAST_INSERT_ID()` lookups unreliable.** Every generated function took
  `\Amp\Sql\SqlConnectionPool`, so a single `MysqlConnection` was rejected outright — but a pool is
  the wrong thing to pass on MySQL: `GetLastInsertUser` resolves `LAST_INSERT_ID()`, which is scoped
  to the connection that ran the `INSERT`, so the pool routes the follow-up `SELECT` to a different
  connection and it finds no row. The parameter is now `\Amp\Sql\SqlExecutor`, the narrowest
  interface carrying the `prepare()` the generated code actually calls; both the pool and the bare
  connection implement it on PostgreSQL and MySQL alike. This is a widening — callers already
  passing a pool are unaffected. Found by running the new `php-amphp-mysql` integration project.

- **`go-database-sql` on DuckDB failed at runtime on every nullable parameter.** The manifest maps a
  nullable parameter to `*{T}`, and `go-duckdb` cannot bind a typed pointer at all: measured against
  v2.3.3, both a nil and a *non-nil* `*string` fail with `could not bind parameter / unsupported data
  type: unknown type`, while an untyped nil and a bare value both bind. So this was never limited to
  NULL arguments — any query with a nullable parameter was unusable. Pointer-typed arguments are now
  dereferenced at the bind site (nil becoming an untyped nil) by a generated helper, leaving the
  public function signature unchanged. The other `database/sql` engines bind pointers natively and
  are untouched. `go-database-sql-duckdb` now runs in the `integration-duckdb` CI job, which is what
  surfaced this. (#228)

- **`ruby-oci8` handed back a LOB locator where the generated row type declared a `String`.**
  OCI8 returns a lazy `OCI8::CLOB` / `NCLOB` / `BLOB` / `BFILE` handle rather than a materialized
  value, so a LOB-backed field held the locator instead of its contents. Both CLOB and VARCHAR2
  resolve to the neutral type `string` (and BLOB and RAW to `bytes`), so nothing at the neutral
  level could tell them apart — the fix dispatches on the column's raw `sql_type`, matching what
  `rust_sibyl.rs` already does for the same problem on the same engine. Applied at all seven
  column-read sites, deliberately **not** to the grouped-query grouping key: a LOB's read position
  hits EOF after the first `#read`, so wrapping the key would blank the field read afterwards.
  Found by `ruby-oci8-oracle`'s first CI run that got far enough to execute queries. Its step is
  restored and now runs last in the Oracle job, so a failure there costs only its own coverage.
  Applied to the `cursor.fetch` reads only: a `RETURNING ... INTO` output bind is declared to OCI8
  up front as `bind_param(n, nil, String)` and OCI8 materializes the value into that class, so
  `cursor[n]` there is already a `String` and wrapping it raised `undefined method 'read' for an
  instance of String` in `create_order`. The test covering that path had asserted the wrapped
  spelling, so it guarded the defect instead of against it; it is inverted. (#225)

- **`ruby-oci8` called a cursor method on an integer for `:exec_rows` and `:exec_result`.**
  `OCI8#exec` is polymorphic in its return: an `OCI8::Cursor` for a `SELECT`, but the number of
  rows processed — a plain `Integer` — for `INSERT`/`UPDATE`/`DELETE`. The generated code bound the
  result and called `.row_count` on it, which is a `Cursor` method, so `delete_orders_by_user`
  raised `undefined method 'row_count' for an instance of Integer`. For DML the count is already
  the return value. Surfaced by the Oracle CI job only after the LOB fix above let it run that
  far. (#225)

- **A harness executing the shared schema could send several statements as one.** Every generated
  harness splits `schema.sql` on `;` and runs the fragments; the split was not SQL-aware, so a
  semicolon inside a string literal or a `$$`-quoted body split mid-statement. All eight templates
  now split with a small state machine that tracks `'...'`, `"..."`, `$$...$$` **and `--` line
  comments** — the last of those is not optional: without it an apostrophe in a comment
  (`schema.sql's`) opens a phantom literal and swallows every following semicolon, which is
  strictly worse than the naive split it replaced. Block comments are not handled, and each
  splitter says so; no schema under `integration_tests/sql/` uses them. (#224)

- **`csharp` and `elixir` harnesses printed `PASS` for a test whose assertions had just failed.**
  Same defect fixed for typescript in this release. `python`, `php` and `ruby` turned out **not**
  to share it — their assertion helpers raise or throw, so the `PASS` line after a failure is
  unreachable — and `go`, `java` and `kotlin` already use a passed/failed counter. In every case
  the run still exited non-zero; the damage was to whoever reads the log. (#227)

- **`elixir-jamdb` generated code could not run at all.** Four independent defects, each hidden
  behind the last, found by giving `elixir-jamdb-oracle` its first CI step and then fixed and
  verified against a local Oracle 21c:

  - Every generated function called `Jamdb.Oracle.query/3` on the value `Jamdb.Oracle.start_link/1`
    returns. That is a `DBConnection` **pool**, and `query/3` sends a `{:sql_query, ...}`
    `GenServer` call only a raw connection process answers, so the first call raised
    `FunctionClauseError`. The backend's own `@spec` already said `DBConnection.conn()`. Queries now
    execute through `DBConnection.execute/3`.
  - `DBConnection.execute/3` returns `{:ok, query, result}`, so every result match gained the
    middle element.
  - **`RETURNING ... INTO` returns rows column-wise** — one single-element list per OUT parameter.
    `INSERT ... RETURNING id, name INTO :2, :3` yields `%{rows: [[1], ["Alice"]]}`, not
    `[[1, "Alice"]]`, so the old `[row | _]` match bound only the first column and destructured it
    across every field. A plain `SELECT` is row-wise, so only the `RETURNING` path transposes.
  - jamdb returns an Oracle `NUMBER` as an Elixir **float** whatever its scale — `1.0` for an
    integer key, `99.99` for a `NUMBER(10,2)` — while the manifest declares those columns
    `integer()` and `Decimal.t()`. `Decimal.equal?/2` rejects a float outright, which is how this
    surfaced; the integer case was quieter and merely wrong. Numeric columns are now converted to
    the type the struct declares.

  `elixir-postgrex` and `elixir-ecto` were unaffected throughout. The CI step is restored, and
  `elixir-jamdb-oracle` now passes end-to-end. (#223)

- **A composite value containing a double quote came back truncated, with every field after it
  shifted.** PostgreSQL's `record_out` escapes a literal `"` inside a quoted composite field by
  *doubling* it (`ROW('he said "hi"', 'back\slash')` renders as `("he said ""hi""","back\\slash")`),
  but every composite text parser scythe emits recognized only the backslash spelling. On a doubled
  quote each one took the first `"` for the field's closing quote — truncating that field's value and
  then resynchronizing on the wrong character, so unrelated later fields silently received wrong
  values. Fixed in all nine emitted parsers (`java-jdbc`, `java-r2dbc`, `kotlin-jdbc`, `kotlin-r2dbc`,
  `kotlin-exposed`, `python-psycopg3`, `typescript-pg`, `typescript-postgres`, `typescript-kysely`);
  the five JVM ones shipped with the defect, the rest inherited it from `java_jdbc.rs` as the model.
  Covered by `composite_text_escaping_regression.rs`, which runs the emitted python parser against
  the exact text PostgreSQL 16 produced. (#204)

- **A nullable composite column decoded to the driver's raw value while the generated type claimed
  otherwise.** `python-psycopg3` and `python-asyncpg` declared `address: UserAddress | None` and the
  three typescript backends declared the composite interface, but all five assigned the driver's raw
  value straight through — a `str` for psycopg3, an `asyncpg.Record` for asyncpg, `undefined` fields
  for the typescript drivers. psycopg3 and typescript now parse PostgreSQL's composite text form
  through a generated `_from_text` / `parse{Name}`; asyncpg reads the `Record` it already decodes
  through `_from_record`. A nullable enum column likewise now reads as `None if raw is None else T(raw)`
  rather than calling the enum constructor on `None`. Verified live against PostgreSQL 16, which is
  how the defect was found in the first place.

  The remaining seven PostgreSQL backends are now fixed too, each according to what its driver
  actually does — established by reading the vendored driver source rather than assuming:
  `elixir-postgrex` and `elixir-ecto` get a `from_tuple` conversion, because Postgrex already
  decodes a composite into a natively-typed positional tuple and never hands back text; `ruby-pg`,
  `php-pdo`, `php-amphp`, `csharp-npgsql` and `go-pgx` get a text parser, because their drivers do
  hand back `record_out` text. `csharp-npgsql` additionally sets `UnknownResultTypeList` for the
  composite column, since Npgsql's native `MapComposite<T>` needs a registration the generated code
  cannot perform on the caller's behalf; `go-pgx` is the same story for pgx's type map. PHP's parser
  is emitted once per file as a shared class rather than copied into each composite. (#204)

  All seven integration harnesses now select a composite column and assert on it — a present value,
  a SQL NULL, a nullable enum, and a field containing `"` and `,`. That last case is the one that
  matters: an assertion using only plain values passes identically against the pre-fix parser, which
  is how the doubled-quote defect survived in nine backends. Confirmed falsifiable by reverting the
  fix in `ruby-pg` and watching the new assertion catch it —
  `expected "12 \"Main\", Apt 3", got "12 "`. (#204, #226)

- **The generated python composite parser did not type-check, and cast a NULL sub-field away.**
  `_from_text` fed `_parse_composite_fields`' `str | None` tokens straight into fields declared
  `str`, which pyrefly rejects — and PostgreSQL does permit a NULL sub-field, so the value really
  can arrive. Silencing the checker with a cast would have traded a type error for a value that
  lies at runtime, so the str-typed fields now route through a `_require_composite_field` guard that
  raises naming the field that was NULL. asyncpg's `_from_record` also gained an `Any` annotation on
  its `record` parameter, which pyrefly rejected outright as unannotated. (#204)

- **A composite whose field named another composite emitted the two definitions in the wrong order.**
  The analyzer discovers composites breadth-first, so a type reached only through another composite's
  field list landed *after* the type that references it. Languages whose declarations hoist never
  noticed; python evaluates `@dataclass` annotations when the class body runs, so the generated module
  raised `NameError` on import. Definitions are now emitted in dependency order. (#204)

- **Four integration projects had their generated output checked for freshness but never executed.**
  `elixir-jamdb-oracle` and `ruby-oci8-oracle` now run in `integration-oracle`, `kotlin-jdbc-ext` and
  `php-pdo-namespace` in `integration-pg`. Each already had a `test:*` Taskfile target and needed no
  new infrastructure — only the missing workflow step. `php-pdo-snowflake` stays exempt, with its
  reason corrected: `pdo_snowflake` ships as neither a PECL nor an apt package and must be built from
  source against Snowflake's C driver, which is infrastructure work rather than a missing step. This
  is the gap that let the csharp-snowflake parameter-binding bug survive from v0.6.0 to 0.14.0. (#118)

- **A semicolon inside a SQL comment broke every harness that executes the shared schema.** Each
  generated harness runs `sql/<engine>/schema.sql` by splitting it into single statements on the
  semicolon character, and that split is not comment-aware — so a semicolon inside a `--` comment
  ended the fragment there and left the rest of the comment line to be sent as bare SQL.
  `elixir-postgrex` failed with `ERROR 42601 syntax error at or near "this"`. Only elixir surfaced
  it, because the postgres job fails fast and the harnesses ahead of it happen not to split that
  schema. The comment is rewritten and a `schema_sql_comments_contain_no_semicolon` generator test
  now enforces the fixture side of the contract; the naive splitting itself is tracked separately.

- **`ruby-oci8`'s teardown called a method that does not exist.** The generated harness ended with
  `conn&.close`, but `OCI8` disconnects via `#logoff` — so the `ensure` block raised
  `NoMethodError` and masked whatever the real failure had been.

- **Generated Ruby raised `LoadError` on Ruby 3.4+.** `bigdecimal` stopped shipping as a default gem
  in Ruby 3.4.0, and `ruby-pg`, `ruby-mysql2` and `ruby-trilogy` emit `require "bigdecimal/util"`
  whenever a query's generated code applies `.to_d` to a `decimal` column — so on 3.4 that `require`
  failed unless something else in the bundle happened to depend on the gem. The generated `Gemfile`
  for those three drivers now declares `bigdecimal` explicitly. `ruby-oci8` declares it too, for a
  second and independent reason: ruby-oci8's own `lib/oci8/bindtype.rb` requires `bigdecimal` lazily
  when it decodes an Oracle `NUMBER` column and does not declare that dependency in its gemspec, so
  reading any numeric column raises `LoadError` regardless of what scythe emits. `ruby-sqlite3` and
  `ruby-tiny-tds` need neither and are unaffected. CI pinned Ruby 3.3 — a version predating the
  change — so it structurally could not observe any of this; the integration workflow now pins 3.4.

- **An enum reachable only through an array column generated with no variants.** The analyzer's
  enum-discovery loop matched the bare `enum::x` neutral type, so a column typed `mood[]` — neutral
  type `array<enum::mood>` — was never recognized as referencing `mood`. `scythe-codegen`, which
  unwraps containers on its own, then found the type reachable but had no `EnumInfo` for it and fell
  back to a stub with an empty variant list, emitting an enum declaration with no variants. (#165)

- **An explicit but empty `[sql.gen]` table silently generated a `rust-sqlx` target.** A legacy
  `[sql.gen]` block naming none of `rust`/`python`/`typescript`/`go`/`kotlin` resolved to a default
  `rust-sqlx` target rather than an error — the same silent-fallback shape #97 removed for an
  *unresolvable* target, left open for a block that resolves to *nothing*. Omitting the `gen` key
  entirely still defaults to `rust-sqlx`, which is documented and intended. (#165)

- **A `derive` backend option repeating a base derive produced code that would not compile.**
  `SqlxBackend::derive_line` appended every `extra_derives` entry unconditionally, so naming `Debug`
  (always in the base set) or `serde::Serialize` alongside `serde = true` emitted a duplicate derive
  token — `E0119`, conflicting trait implementations, in the generated file. (#165)

- **A typo'd case name in a `[naming]` manifest overlay installed silently.** `apply_case` passes an
  unrecognized case name through unchanged, which is safe for the compiled-in manifests but not for an
  overlay, the one path a case name reaches it from outside. `struct_case = "PascalCse"` was accepted
  and then emitted every affected identifier uncased. Overlays are now validated against the four real
  case names. (#165)

- **`scythe lint <file>` ignored `[lint.sqruff]`.** Explicit-file mode built its sqruff linter with
  `None` in place of the config's rule table, unconditionally — the same gap #206 closed for `fmt`,
  left open in `lint`. A `[lint.sqruff]` that config-mode `lint` rejects was silently accepted when
  the same config was paired with a file argument. (#206)

- **A `column = "table.col"` override that could only ever match a *parameter* was never flagged.**
  The unmatched-override preflight built its known-references set from columns alone, so a qualified
  override naming a real parameter reference but no column passed silently — as did a typo'd one,
  since neither could be distinguished from an override that simply never fires. `resolve::param_references`
  is now chained into the same set, feeding the existing diagnostic rather than adding a second. (#189)

- **`SC-PRV09`, `SC-PRV10`, `SC-PARSE01` and `SC-PARSE02` could not be configured, counted, or
  discovered.** All four were ad hoc `Error`-severity findings `scythe-cli` constructed directly at
  the point of failure (an unconstructable `[[sql.gen]]` target, a query file with zero recognized
  blocks, a query that fails to parse, a query that fails semantic analysis), never as registered
  `LintRule`s — so `[lint.rules]` and `[lint.categories]` had no effect on any of the four, and the
  documented "8 provenance rules" undercounted the 11 that actually exist by two. `SC-PRV09`
  (`gen-target-invalid`) and `SC-PRV10` (`empty-query-file`) now join `SC-PRV01`-`08`/`SC-PRV11` in
  `scythe_lint::provenance_registry`; `SC-PARSE01` (`unparseable-query`) and `SC-PARSE02`
  (`unanalyzable-query`) get a new `scythe_lint::parse_registry` and `RuleCategory::Parse`, since they
  fire from `check`, `lint`, and `audit` alike rather than a single check-time command. All four are
  zero-behavior `LintRule`s exactly like the rest of the provenance family: the finding itself is
  still built where the failure is detected, but its severity is now resolved from the registry
  instead of hardcoded. (#216)

- **A schema-qualified enum or composite generated two different names for the same type.** The
  declaration side spelled the type through `enum_type_name` / `composite_type_name`, which strip
  characters an identifier cannot hold; the reference side — the type as it appears in a column,
  parameter or composite-field annotation — called `to_pascal_case` directly. So
  `CREATE TYPE app.point` was declared as `AppPoint` and referred to as `App.point`, a `.` inside
  a type position that no target language parses, and the reference never matched the declaration
  it named. Both paths now share one helper. The same call also hardcoded PascalCase instead of
  honouring the manifest's `struct_case`; that half was latent only because all manifests currently
  set PascalCase, and is fixed alongside. Separately, the composite declaration itself inlined
  `to_pascal_case(&composite.sql_name)` in roughly sixty backend call sites rather than sharing one
  place, including five nested-composite reference sites that disagreed with their own declaration.
  (unfiled)

- **A composite reachable only as another composite's field was never emitted.** The analyzer
  collected composites by scanning selected columns and nested field types, and never looked inside
  a composite's own fields, so selecting a column whose type nests another composite produced code
  referring to a type that was never defined. Codegen gated emission on the same incomplete check,
  so collecting it in the analyzer alone would not have been enough. Both now walk the full
  reachability closure, with a visited set that also serves as the diamond and cycle guard. This
  was documented as a known gap when the JVM composite reader landed; it is now closed. (unfiled)

- **A qualified `column = "table.col"` type override was silently ignored for parameters outside
  `SELECT *`.** The per-parameter match key was built from a query-level table name that only ever
  exists for a single-table `SELECT *`, so on any explicit select list the override matched nothing
  and was dropped without a word — the parameter half of the defect whose column half was fixed
  earlier. Parameters bound by a direct `col op $N` comparison now carry their own owning relation,
  taken from the real table name rather than an alias. Parameters with no single owning column (an
  `IN` list, a `LIKE` pattern, a literal comparison) deliberately carry none and keep their previous
  behaviour rather than guessing. (#189)

- **Every JVM backend read a composite column through `getObject(col, T.class)`, which throws at
  runtime.** pgjdbc registers no type map for a user-defined composite, so `PSQLException:
  conversion to class T ... not supported` was raised the first time any generated JVM reader
  touched one — code that compiled and then failed on first use. Composites now read as text and
  parse through a generated `fromText` factory implementing PostgreSQL's composite text-form rules:
  an empty unquoted field is NULL, a field needing quoting is wrapped in `"` with `"` and `\`
  backslash-escaped inside, and a nested composite arrives quoted and recurses. Five assertions that
  pinned the broken `getObject` shape as correct were inverted. Still unhandled and documented
  rather than dropped: array-typed composite fields, per-field NULL into a primitive-typed field,
  and a composite reachable only as another composite's field, which the analyzer never collects.
  (unfiled)

- **`rust-tokio-postgres` could not bind or read a composite column at all.** The generated struct
  derived neither `ToSql` nor `FromSql`, so `row.get` and the bind path both failed to resolve.
  Composites now derive `postgres_types::ToSql`/`FromSql` with `#[postgres(name = "...")]`, since
  postgres-derive matches the Postgres type name exactly while scythe PascalCases the identifier.
  `postgres-types` is a transitive dependency of `tokio-postgres`, but its `derive` feature is not
  forwarded, so it is now declared directly in the integration scaffolding. (unfiled)

- **A `column = "table.col"` type override was a silent no-op unless the query was `SELECT *`.**
  Column resolution built one qualified name from a query-level source table populated only for a
  star expansion, so an explicit select list had nothing to qualify against. Columns now carry their
  own source relation — the real table name, not the alias — and `None` where there genuinely is
  one (a computed expression, literal, or function result). Two silent halves went with it: a
  combined `column` + `db_type` entry returned `false` the moment `column` missed instead of falling
  through to `db_type`, and an override matching nothing produced no diagnostic whatsoever. It is
  now a hard error before generation starts. A qualified override on a *parameter* is still inert
  outside `SELECT *`; that needs analyzer work and is tracked, not quietly half-fixed. (#189)

- **A schema-qualified table emitted a `.` inside its model struct name.** `SELECT * FROM
  app.widgets` produced `pub struct App.widget`, the same defect fixed for enums earlier. Row struct
  names are unaffected — `@name` is already restricted to ASCII identifier characters, verified
  rather than assumed. Separately, two different queries in one output file whose generated types
  collapse onto one identifier emitted two declarations of that name; collisions are now keyed by
  name *and* rendered body, so the identical-body case remains the intended dedupe. Composite struct
  names still carry the dot bug across 56 inlined call sites and are tracked separately. (#136)

- **`scythe migrate` passed a malformed annotation straight through and still reported success.** A
  wrong-case return-type keyword, a missing return type, or whitespace inside `sqlc.arg( name )`
  missed the strict pattern and was emitted unconverted. Malformed input is now reported, and the
  final output is scanned for residual `sqlc.arg(`/`sqlc.narg(`. (#152, partial)

- **`scythe lint` and `scythe audit` accepted an unknown `[[sql]] engine` and silently analyzed it as
  PostgreSQL.** `SqlDialect::from_str(&engine).unwrap_or(SqlDialect::PostgreSQL)` in both
  `lint_cmd.rs` (config mode and explicit-file mode, two separate call sites) and `audit.rs`
  (config mode) turned a typo like `mysql8` into a silent PostgreSQL run — wrong catalog parsing,
  wrong dialect-gated rule set, no diagnostic — while `scythe generate` already rejected the same
  config outright. A new `scythe_lint::parse_engine_dialect`, sharing one alias list with
  `audit --dialect`'s existing validation, now errors naming the offending value and the accepted
  aliases. (#165, item 3)

- **`scythe check` passed on stale output after a `[[sql.gen]]` option changed.** The provenance
  header fingerprinted the schema and the queries but not the options that decide what is generated
  from them, so switching `row_type` from `pydantic` to `msgspec`, or editing the contents of a
  manifest overlay, left the header byte-identical and `check` reported the artifact fresh. A sixth
  `options=` field now covers the target's resolved `[[sql.gen]]` options together with the *contents*
  of its manifest overlay, and `SC-PRV11` reports a mismatch as its own finding rather than folding
  into the existing header rules. A header written before this field existed is still read as
  complete — absence means "generated by an older scythe", not "drifted" — and a target with no
  options and no overlay produces bytes identical to the old five-field header. All 111 committed
  integration artifacts are regenerated to carry it. Fingerprinting uses FNV-1a rather than the
  `ahash` used elsewhere: ahash's "fixed" keys are regenerated per process from OS randomness, so it
  cannot produce a value that is stable across the write and the later verify. (#155)

- **Two queries selecting the same composite column emitted its model struct twice.** Enum
  definitions were already deduplicated when assembling an output file; the composite/model structs
  beside them were not, so a second query selecting the same composite produced a duplicate type
  declaration — a compile error in every target with a one-definition rule. Deduplicated on the
  rendered struct text, the same way enums already are, and not scoped to the JVM. (unfiled)

- **A schema-qualified enum emitted a `.` inside the generated type name, and two names colliding in
  one file went undetected.** `CREATE TYPE app.status AS ENUM (...)` carries its qualifier into
  `EnumInfo::sql_name`, and case conversion alone does not remove it, so `app.status` became
  `App.status` — `pub enum App.status`, a syntax error in every target that shares this path. Enum
  type names now go through the same `sanitize_for_identifier` the variant labels already used.
  Separately, `to_pascal_case` returned the empty string when every `_`-delimited part was empty (a
  bare `"_"`, or the underscore run a symbols-only label sanitizes to), emitting a type with no name;
  it now falls back to its sanitized input, matching what `to_camel_case` already did. Two generated
  *type* names that collapse onto one identifier — two enums, or an enum and the query's own row type
  — are now rejected with `DuplicateAlias` instead of emitting two declarations of the same name.
  (#136)

- **Parameters were bound by declaration order rather than by where they appear in the SQL, so a
  repeated or out-of-order placeholder bound the wrong argument.** `java-jdbc`, `kotlin-jdbc`,
  `kotlin-exposed` and `php-amphp` emitted one `?` per *declared* parameter and then set them
  `1..n` in declaration order. A query writing `$2` before `$1` therefore bound the caller's first
  argument to the second slot — silently wrong results, no error — and a query repeating `$1` emitted
  fewer binds than the rewritten SQL contained, which the driver rejects at execute time. Placeholder
  rewriting now returns the sequence of parameter positions it actually emitted, and each backend
  binds from that sequence. `preprocess_oracle_sql` and `preprocess_mssql_sql` were also collapsing
  `:N` / `@pN` to a bare `?` before parsing, discarding which N each referred to; they now emit `$N`,
  which sqlparser's `OracleDialect` and `MsSqlDialect` both tokenize as `Token::Placeholder` through
  the default `supports_dollar_placeholder` impl. (#149)

- **SQL-text cleanup and placeholder rewriting were dialect-blind, corrupting MySQL and MSSQL
  identifiers.** The comment stripper and placeholder rewriter knew only PostgreSQL quoting, so a
  MySQL backtick-quoted identifier containing `--` had the rest of the query deleted, a MySQL `#`
  line comment was left in place, and an MSSQL `[bracketed]` identifier containing a comment marker
  was truncated the same way. `SqlDialect` is now threaded through the whole SQL-text pipeline, and
  backtick and bracket spans are recognised as quoted regions alongside PostgreSQL's. Bare `?` under
  PostgreSQL was previously governed by a heuristic — rewrite `?` only if the query contains no
  `$<digit>` anywhere — which corrupted a zero-parameter query using the JSONB `?` operator; the
  decision is now made from the dialect instead of from a scan of the text. This is the last of
  #186's items; the JSONB `?` operator, dollar-quoted strings and `NOT LIKE` were fixed earlier.
  (#186)

- **`:one` and `:opt` rendered identical code on 53 backends, so one of the two contracts was always
  silently wrong.** `:one` means "exactly one row, error if absent"; `:opt` means "zero or one". Every
  affected backend matched `QueryCommand::One | QueryCommand::Opt` in a single arm, so whichever
  behaviour that arm happened to implement won for both. Each language now gets an error path built
  from its own idiom — a raised `ScytheNoRowsError` / `RecordNotFound` / `RecordNotFoundException`, a
  thrown `NoSuchElementException` or `InvalidOperationException`, `Mono.error` on the reactive
  backends, the driver's own `ErrNoRows` in Go, `{:error, :not_found}` in Elixir, and `Err` in Rust —
  while `:opt` keeps its existing shape everywhere. Ruby `.rbs` signatures and PHP return-type
  declarations were narrowed to match, so signatures no longer over-promise nullability. (#197)

  The direction was not uniform, and the earlier census recorded it wrongly for 10 of the 53. On the
  `go-*` and `elixir-*` backends `:one` was already correct — `sql.ErrNoRows` propagates through
  `row.Scan`, and Elixir already returned `{:error, :not_found}` — and it was `:opt` that wrongly
  errored on a legitimately absent row. `go-godror` folded the permissive way while its three Go
  siblings did not, so even same-family behaviour was not safe to assume.

- **`python-snowflake` declared `:execrows` as `-> int` while returning `cur.rowcount`, which the
  DB-API types `int | None`.** Narrowed at the call site rather than widening the annotation:
  psycopg, aiosqlite, aiomysql and oracledb all type `rowcount` as plain `int`, so snowflake was the
  lone outlier and widening would have spread the imprecision to seven backends. (unfiled)

- **`typescript-postgres` could not bind a composite-typed parameter.** postgres.js serialises only
  values it recognises, and a plain object standing for a PostgreSQL composite is not one, so the
  generated tagged template failed to type-check. Composite parameters are now expanded to
  `ROW(${field}, ...)::type_name` — one binding per scalar field, recursing through nested composites
  — instead of being interpolated whole. (unfiled)

- **A `rust-sqlx` `:grouped` query selecting a non-identifier column produced code that could not
  compile.** The grouped path reads its flat rows through the untyped `sqlx::query!` macro, whose row
  field names come from sqlx's own expansion of the raw column names rather than from this backend's
  `sanitize_field_names` convention. sqlx's `parse_ident` requires the driver-reported name to be a
  valid Rust identifier and otherwise fails macro expansion outright, so a quoted `"my col"` was a
  hard compile error, not a silent mismatch. Such columns now get an explicit `AS "field_name"` so the
  macro sees a name scythe chose. Note this is a different mechanism from the `#[sqlx(rename)]`
  attribute added earlier for `FromRow`: both `query!` and `query_as!` build their row type directly
  and never consult `FromRow`, so that attribute has no effect on either macro path. (unfiled)

- **The same `rust-sqlx` defect was live on the plain `:one`/`:many`/`:opt` path, and its enum
  aliasing emitted a stray backslash into the SQL.** `generate_query_fn` selected a non-identifier
  column unaliased, so `parse_ident` failed macro expansion there too; and a column whose name is a
  valid identifier but differs in shape from this backend's `field_name` (case, or
  `sanitize_field_names` reshaping) failed against a struct-literal field spelled differently, since
  `quote_query_as` builds `#out_ty { #ident: #var_name }` from the driver-reported name. Separately,
  `rewrite_sql_for_enums` hand-wrote its alias as `\"…\"` in Rust source — a literal backslash and
  quote — and then passed it through `escape_rust_string`, which escaped both again, so the SQL sqlx
  saw at compile time contained a backslash nobody asked for. Both paths now share one
  `rewrite_sql_for_row_columns`, which aliases whenever `field_name` differs from the column name or
  an enum override applies, writes the alias as a single plain `"…"`, and is escaped exactly once.
  (unfiled)

- **`check` printed "All queries valid." for a query file it had not checked at all.** A file whose
  annotations were never recognised — a mistyped `--name:`, or every statement commented out — yields
  zero query blocks, and `has_unannotated_sql` deliberately ignores it, so the run reported success
  having examined nothing. A non-empty file that produces no query blocks is now an `SC-PRV10` error
  naming the file. A genuinely empty or whitespace-only file is still accepted: there is nothing there
  that could have been misrecognised. (unfiled)

- **`VARBINARY(MAX)` resolved to the invalid neutral type `varbinary(max)`, which no manifest maps.**
  SQL Server's unbounded binary type parses to `DataType::Varbinary(Some(BinaryLength::Max))`, for
  which `normalize_data_type` had no arm at all, so it fell to the catch-all that stringifies through
  `Display`. `strip_precision` only strips a trailing `(<digits>)`, so `max` survived, never matched
  the bare `varbinary` arm, and the column resolved to a type name rather than `bytes`. The sibling
  `VARCHAR(MAX)`/`NVARCHAR(MAX)` spellings were already correct — their arms route
  `CharacterLength::Max` through a `_ => "text"` fallback — and all ten mssql-capable manifests
  already mapped `bytes`, so no manifest changed. `BINARY` needs no equivalent arm: sqlparser types
  it `Option<u64>`, making `BINARY(MAX)` unrepresentable. (unfiled)

- **A literal `%` in SQL broke every `%`-paramstyle Python driver at execute time.** `WHERE name LIKE
  'a%'` reaches psycopg3 and aiomysql as a format string, and `%'` is not a valid placeholder, so the
  driver raised before the statement was ever sent. The `%` is now doubled — but only for a query that
  actually binds parameters. psycopg3 and PyMySQL run `%`-formatting exclusively from
  `execute(query, params)`; a parameterless `execute(query)` passes the string through untouched, so
  doubling it there would have replaced a driver-side error with a silently wrong `LIKE 'a%%'` that
  matches nothing. `python-snowflake` additionally emits
  `snowflake.connector.paramstyle = "qmark"` to match the `?` it generates — previously the only
  `paramstyle` assignment in the tree was a hand-written compensation inside the integration harness,
  so every consumer of the generated module got none. (#201)
- **`python-aiomysql` rewrote a `?` inside a SQL string literal.** A blind `.replace('?', "%s")` ran
  *after* the literal-aware `rewrite_pg_placeholders`, so `WHERE note = 'really?'` became
  `'really%s'` — a silent wrong answer, not an error. GH #153 was closed with this half unfixed.
- **`scythe lint <file>` ran no scythe rules at all.** Explicit-file mode built a sqruff linter and
  never constructed a `LintEngine`, so every `SC-*` rule was skipped — `scythe lint queries.sql`
  silently checked far less than `scythe lint` with the same config, and the code said so in a
  comment. It now builds a catalog from the config's first `[[sql]]` block and runs the native rules
  with suppressions honoured, falling back to sqruff-only when there is genuinely no schema to build
  from. `scythe fmt <file>` likewise dropped `[lint.sqruff]` entirely, honouring only the dialect
  half of #206.
- **`scythe check` green-lit an `output` path that `scythe generate` refuses.** `check` never applied
  #207's containment rule, so a config could pass the check and then fail the thing the check exists
  to predict. (#206, #207)
- **A `CREATE VIEW` with an explicit column list got no types at all.** The branch handling
  `CREATE VIEW v (a, b) AS SELECT …` never ran the analyzer: `sql_type` fell back to the literal
  string `"unknown"` and `nullable` was hardcoded `true`, so the same view declared with and without
  a column list produced different — and wrong — columns. It now analyzes the body and overlays the
  declared names. A declared list whose arity disagrees with the body is now an error rather than
  silently mismatched output, mirroring how a `WITH t(a,b) AS …` alias list is already handled.
- **`ALTER TABLE … RENAME TO` on an unknown table did nothing, silently.** Every sibling operation —
  `AddColumn`, `DropColumn`, `RenameColumn`, `AlterColumn`, `AddConstraint` — errors on a missing
  table; `RenameTable` alone fell through with no `else`, so a typo'd migration was indistinguishable
  from a correct one. It now follows the same precedent.
- **`json_each` and friends were typed `string` in select-list position.** They return `SETOF record`,
  not text. The neutral type vocabulary cannot name an anonymous record — `composite::{name}` needs a
  catalog entry and the `json_nested` machinery assumes the value on the wire is JSON text, which a
  native composite is not — so they now resolve to `unknown`, following the precedent
  `json_populate_record` already set, rather than to a confidently wrong scalar.
  `json_array_elements`/`jsonb_array_elements` previously hit the unknown-function error path and now
  resolve to `json`, matching what the FROM-position handling already assigned.
- **`rust-sqlx`'s `:opt` output never compiled.** The return type said `{Struct}` while the body's
  `has_row_struct` guard excluded `Opt`, so it emitted the anonymous-record `sqlx::query!` instead of
  `sqlx::query_as!` — the declared type and the produced type disagreed on every `:opt` query the
  backend has ever generated. `:opt` now returns `Option<{Struct}>` and fetches with
  `.fetch_optional`, which is what the command means. `rust-tiberius`'s `:opt` likewise stopped
  emitting `.expect("expected one row")`, a panic in generated code on exactly the absent row `:opt`
  exists to handle. (#197)
- **`rust-sqlx` mapped a mangled field back to the wrong column.** The backend derives
  `sqlx::FromRow`, which looks a column up *by the Rust field name*, and #215's
  `sanitize_field_names` renames any non-identifier column — so `my col` became a field `my_col` that
  `FromRow` then searched for under that name and could not find. A compile fix bought at the cost of
  a runtime one. Fields whose generated name differs from the SQL column now carry
  `#[sqlx(rename = "…")]`. The other Rust backends were checked and are unaffected: tokio-postgres
  and tiberius look up by the raw SQL name, sibyl reads positionally.
- **`typescript-duckdb` typed a `bytes` column as something the driver never returns.** The manifest
  declared `Uint8Array`; `@duckdb/node-api` hands a BLOB back as `DuckDBBlobValue`. Verified against
  the published package rather than inferred — 1.5.5-r.4 ships
  `class DuckDBBlobValue { readonly bytes: Uint8Array }` and lists it in the `DuckDBValue` union. The
  read direction had no test at all, which is why this survived. Note the manifest has no read/bind
  split, so the bind-position type changed too: construct one with the driver's
  `blobValue(Uint8Array | string)`.
- **The tool-validation schemas contained no container or user-defined type.** The ~20 PostgreSQL
  backend tests that compile generated code with a real compiler — the strongest gate in the project
  — never asked one to accept an array, an enum, an array of enums, a composite, a `uuid` or a
  `jsonb` column, which is why the JSDoc and JVM enum defects above survived. The schemas now carry
  all of them, and every added column is selected by the query each test runs; a widened schema with
  an unwidened query would have added columns no generated file reaches. MySQL gains an inline
  `ENUM(...)` column for the same reason. (#146)
- **The two `SC-INS09` live tests raced each other.** Both trusted `CREATE EXTENSION IF NOT EXISTS`
  to tell them where an extension landed; they now verify against `pg_extension`/`pg_namespace`. (#144)
- **`SC-N02` (`table-naming`) could not see a CamelCase table name.** The catalog stores tables under
  a lowercased lookup key, so by the time the rule read the name every table looked snake_case and
  `CREATE TABLE "UserProfile"` passed. `Table` now keeps the DDL's own spelling in `raw_name`
  alongside the lookup key, and the rule reads that. The existing test asserted the miss verbatim —
  it guarded the bug rather than against it — and is inverted here. (#145)
- **A placeholder inside a `LIKE` pattern or an `IS NULL` operand was dropped from the generated
  signature.** `WHERE name LIKE '%' || $1 || '%'` and `WHERE $1 IS NULL` both bind a parameter, but
  the analyzer only collected one from a `LIKE` whose pattern was a bare literal and never descended
  into `IS NULL` / `IS NOT NULL` at all — so the generated function took fewer arguments than the
  statement needs. Placeholder positions are now memoised by source span, which keeps a parameter
  repeated across several expressions from being counted more than once. (#171)
- **`go-pgx` emitted a static import block that omitted imports its own types needed.** A query
  selecting a `json` or `uuid` column produced `*json.RawMessage` and `uuid.UUID` with neither import,
  so the file did not compile; the generated header conceded as much by advising `goimports -w .`.
  #100 fixed the opposite direction — an import emitted but unused. Imports are now derived from the
  types actually emitted, via the `[imports.rules]` table every Go manifest already declared and
  nothing read. `go-pgx` consequently passes the torture gate and has been removed from the
  expected-failure allowlist. The PHP casts likewise come from the manifest instead of a hardcoded
  table that contradicted it. (#198)
- **A JVM enum whose SQL spelling was not the uppercase of its variant threw on every read.** Binding
  emitted `.getValue()` / `.value` — the SQL value — while reading emitted
  `valueOf(rs.getString(col).toUpperCase())` — the variant *name*. For a value like `in-active` with
  variant `IN_ACTIVE`, `toUpperCase()` yields `IN-ACTIVE`, which `valueOf` rejects with
  `IllegalArgumentException`; case-folding is not the same operation as sanitising. The generated
  `value`/`getValue()` accessor that makes this exact was emitted and consulted by no reader. Reads
  now match on the declared SQL value. The existing tests asserted the `toUpperCase()` spelling
  verbatim, so they pinned the defect and changed with the fix. (#213)
- **Every `javascript-*` file containing an enum failed `tsc --checkJs`.** The generated
  `/** @type {const} */` sat on the declaration, where it is `TS2304: Cannot find name 'const'`. The
  valid position is the initializer expression — `= /** @type {const} */ ({…})` — which also narrows
  to literal types as intended. The same spelling existed as three byte-identical copies across
  `typescript-pg`, `typescript-postgres` and `typescript-mysql2`; they now share one
  `generate_js_enum_def`, so the next fix here lands once instead of three times.
- **A non-identifier column name was spliced raw into a JSDoc `@property`.** `@property {string} my
  col` is `TS1003: Identifier expected`, and unlike the TypeScript emit path the JSDoc row typedef
  cannot mangle the name — a generated row type is cast onto the driver's rows, so its key must stay
  the column's own spelling. The typedef now switches to JSDoc's quoted type-literal form
  (`@typedef {{ "my col": string }}`) when any key is not a bare name, and keeps the `@property` form
  otherwise. `@param` mangling is unaffected and remains correct: a binding is a JavaScript
  parameter, which has no quoted form.
- **`javascript-better-sqlite3`'s `:batch` path never type-checked.** `db.transaction((items) => …)`
  with an unannotated parameter makes TypeScript infer `never` from better-sqlite3's variadic
  signature, giving TS2488 and TS2345. The TypeScript emit path annotates it as `(items: T[])`; the
  js_mode path now carries the equivalent inline `@param`. This was invisible until the enum fix
  above stopped `tsc` short-circuiting on an earlier error.
- **`typescript-postgres`'s single-parameter `:batch` rewrote `${field}` inside a SQL string
  literal.** A blind `String::replace` matched the tail of an escaped `\${field}`. #219 covered the
  `$N` form only.
- **A `:grouped` query's `.rbs` described a class the `.rb` file never defines.** The RBS producer
  resolved the flat column list where the Ruby producer splits parent from child, so the signature
  declared one class with neither the `children` reader nor the child class `Data.define` actually
  emits — `steep check` against a correct `.rb` failed on the signature, not the code. The RBS path
  now performs the same split. `RbsQueryInfo` carries the child columns in their own field; an
  earlier revision smuggled them through a sentinel in `ResolvedColumn.full_type`, which is the shape
  that leaked `__unknown_col__` into user-visible output in #173. (#203)
- **Eight `.rbs` files were rejected outright by `rbs parse`.** The Ruby backend emitted
  `library "bigdecimal"` ahead of any signature referencing `BigDecimal`, but `library` is Steepfile
  and CLI syntax, not an RBS declaration, so the parser failed at the first token with "cannot start
  a declaration". A signature naming `BigDecimal` needs no directive at all. The `.rb` side keeps its
  `require "bigdecimal/util"`, which is genuinely needed for `.to_d`.
- `elixir-exqlite` releases its prepared statement on every exit path, not just the success one;
  `elixir-tds` types `bytes` and `time`/`time_tz` parameters instead of falling through to `:string`;
  and a `:grouped` query with no parent columns no longer emits `defstruct [, :children]`, which is
  not valid Elixir. (#202)
- **A column named `my col`, `with-dash` or `2fa` reached a field declaration verbatim in every
  language but TypeScript.** `pub my col: String`, `my col: str`, `My col string`, `String my col` —
  none of them parse, and no gate caught it because the torture schema has no such column. #215 fixed
  this for TypeScript by quoting (`"my col": string`, `row["my col"]`), which is the right answer
  *there* and only there: a generated TypeScript row type is cast onto the driver's rows, so its key
  has to stay the column's own spelling. The other nine targets have no quoted form for a field and
  never read a column back by the generated name — they use the position or the raw SQL name
  (`rs.getString("my col")`) — so their 85 manifests now set `[naming] sanitize_field_names`, and
  `field_name` replaces the characters an identifier cannot hold. A leading digit takes a `col_`
  prefix rather than a bare `_`, because `to_pascal_case` drops a leading underscore and go-pgx and
  the C# backends case the field name a second time, which handed the digit straight back. The SQL
  text is untouched. (#215)
- **`IDENTITY` preprocessing ate the whitespace after the keyword and rewrote its case.** The catalog
  strips `IDENTITY(seed, step)` before parsing; when the keyword was *not* followed by a clause, the
  branch that put it back pushed a literal uppercase `"IDENTITY"` and resumed from the position it had
  already advanced past the whitespace, so `GENERATED ALWAYS AS IDENTITY PRIMARY KEY` became
  `IDENTITYPRIMARY KEY` and a column named `identity` became `IDENTITYTEXT NOT NULL`. The original
  characters are now copied through unchanged. Thanks to @fzlzjerry. (#154)
- **An unsupported nested `json_agg` degraded to a single JSON object even where the driver could
  describe the array.** The degradation pass rewrote every column referencing a nested struct the
  backend did not implement to plain `json`. A distinct `json_array` scalar marker now carries "one
  JSON document whose top level is an array", and `typescript-pg`, `python-asyncpg`, `elixir-postgrex`
  and `php-pdo` opt into it by declaring it in their manifests. It is deliberately not the `array<json>`
  container, which means a SQL `json[]` column and can select a typed array reader: `csharp-npgsql`
  would have declared `List<string>` while reading through the untyped `GetValue` accessor. Backends
  that declare no `json_array` mapping keep the plain-`json` fallback unchanged. Thanks to @fzlzjerry.
- **A parameter named after a column like `my col`, `with-dash` or `2fa` was emitted verbatim into a
  binding.** `export async function findWeird(client: PoolClient, my col: string)` does not parse, and
  neither does its equivalent in the other nine target languages. #215 fixed the two positions that
  have a quoted form — the declared property key and the property read — and left this one pinned as a
  known gap, because a binding has no quoted form anywhere and mangling is a cross-language naming
  decision. `scythe_backend::naming::param_name` now makes it: characters an identifier cannot hold
  become `_`, and a leading digit takes a `_` prefix. Only parameters are mangled. A column's field
  name is a contract with whatever the driver returns, so it keeps its raw spelling and its quoting;
  the SQL text is untouched, and any collision the mangling introduces (`my col` against a real
  `my_col`) is reported by the existing duplicate-field check rather than silently resolved. (#215)
- **`python-psycopg3` bound a reserved-word parameter to a name it never passed.** It is the one
  backend that binds by name rather than position, and it derived the two halves of that contract
  separately — the `execute` dict from the resolved param's `field_name`, the `%(...)s` placeholder
  from a second `to_snake_case` of the raw SQL name. The spellings matched until anything else touched
  `field_name`: a param named `class` became `class_` in the signature and the dict while the SQL still
  asked for `%(class)s`, so every call raised `query parameter missing: class` at execute time. Nothing
  earlier could catch it — the module imports, type-checks and passes the generated-code gate, which
  compiles generated code and never runs it. Both halves now come from the resolved param, the way
  `typescript-postgres` already did it. `crates/scythe-codegen/tests/python_named_placeholder_regression.rs`
  asserts the invariant (every placeholder is a dict key) rather than the single keyword that exposed it.
- **A column named after a TypeScript keyword produced a file that would not parse.** Every generated
  TypeScript query function takes its parameter names from the columns they are compared against, so a
  `class` column emitted `export async function q(client: PoolClient, class: string)` — `TS1390`, and
  the syntax error stopped `tsc` before it type-checked anything else in the file. The seventeen
  TypeScript manifests now declare `[naming] reserved_bindings`, consulted by a new
  `scythe_backend::naming::param_name`, which mangles a keyword to `class_` where it lands in a
  binding. Deliberately *not* the existing `[naming] reserved` list: that is applied to columns too,
  and a generated TypeScript row type is cast straight onto the driver's rows
  (`client.query<FindByClassRow>(...)`), so renaming the key would have described an object `pg` never
  returns — a compile error traded for a silent wrong answer. `class` therefore stays `class` in the
  row type and in `row.class`, both of which are legal TypeScript. Five of the six TypeScript entries
  in `scripts/torture-expected-failures.txt` are gone; `typescript-postgres` still fails, on a
  composite-typed parameter postgres.js cannot bind, which was invisible behind the syntax error.
  (#180)
- `scripts/check-generated-backends.py` ran `ruby -c` over `queries.rbs`. The script globs every file
  in a backend's output directory and picked the syntax checker by *backend*, but RBS is a signature
  language, not Ruby, so it choked on `ACTIVE: String` and reported a `ruby-pg` failure that no change
  to the generated code could ever have cleared. The entry sat in
  `scripts/torture-expected-failures.txt` blamed on unescaped SQL, which it never had anything to do
  with. Syntax checkers are now selected by file extension first, since a file's language is a property
  of the file and not of the backend that emitted it — the distinction
  `scripts/check-generated-syntax.sh` already made, now with one derivation instead of two. `ruby-pg`
  builds clean against the torture schema and is out of the allowlist.
- Every remaining reason in `scripts/torture-expected-failures.txt` was re-derived from the compiler's
  actual output rather than carried forward. All five non-TypeScript entries had been grouped under
  "unescaped quoted identifier (#179)", written mid-rollout and wrong for every one of them by the time
  740cc99 finished the escaping layer: the three Rust projects fail because their scaffolding declares
  neither `serde_json` nor `uuid`, `go-pgx` fails on a static import block that omits what its own
  emitted types reference (#198), and `ruby-pg` was the harness bug above. A gate that checks only
  pass/fail cannot check *why*, so the file now carries an instruction to re-derive before editing.
- `SC-SEC01` (`dangerous-function`) missed set-returning functions called in `FROM` position
  (`FROM dblink(...)`, `FROM pg_ls_dir('/etc')`, `FROM openrowset(...)`) — the idiomatic way these
  particular functions are written — because the matcher only inspected `Expr::Function` nodes and
  `pre_visit_relation` was a no-op. It now also matches the relation name. (#138)
- `SC-SEC06` (`weak-hash-in-auth`) missed salted and wrapped hash arguments: `md5(password || salt)`
  and `md5(lower(password))` produced no finding, only the bare `md5(password)` form did.
  `extract_sensitive_column` now recurses through `BinaryOp`, `Nested`, `Function` and `Cast`. (#138)
- `SC-A03` (`or-in-join-condition`) only fired when the `OR` in a JOIN's `ON` clause was
  unparenthesised, and only inspected the ON clause's root expression instead of descending it — so
  `ON (a OR b)` and `ON x AND (a OR b)`, both real occurrences of the same antipattern, produced no
  finding. It now unwraps parentheses and descends through `AND` conjuncts, counting each top-level
  disjunction once. (#145)
- `engine.rs`'s cross-query duplicate-name check (`SC-C03`) hardcoded `Severity::Error` regardless of
  `[lint.rules]`, so `"SC-C03" = "warn"` had no effect, and it fired even when `DuplicateQueryNames`
  was not registered in the calling registry at all. It now resolves severity through the registry,
  same as every other rule, and produces no finding when the rule isn't active. (#137)
- `SC-A02` (`implicit-type-coercion`) implements no check and is off by default with no way to ever
  produce a finding if enabled; its description now says so explicitly, matching `SC-C01`. (#137)
- Inline suppression comments (`-- scythe-audit: ignore[...]`) were keyed by source line, so two
  statements sharing one physical line (`DROP TABLE a; DROP TABLE b;`) resolved to the same key and a
  suppression meant only for the first silently covered the second too. `SuppressionSet` is now keyed
  by 0-based statement index instead — **callers must pass a statement index, not a computed source
  line**. The module doc's claim that a blank line between an annotation and its statement still
  attaches was also wrong; the code discarded it then and still does, so the doc was corrected instead
  of the (intentional) behavior. (#140)
- A user-supplied `[[audit.rule]]`'s declared `cwe` array had no way to reach a caller through
  `LintRule` — only `MatcherRule`'s private `RuleSpec` held it, so every consumer fell back to
  scanning `description` for `CWE-\d+` text and a declared `cwe` with no such text in its description
  was silently dropped. `LintRule` gained a `cwe()` method (default: empty; `MatcherRule` prefers the
  declared `cwe`, falling back to the description scan only when it's empty). (#140)
- `MatcherRule::check_query`'s dialect gate (`spec.dialects`) was invisible from outside — a rule
  scoped to Postgres just silently returned nothing on every other engine, indistinguishable from a
  rule that ran and found nothing. `LintRule` gained an `is_applicable_to(dialect)` method (default:
  every dialect; `MatcherRule` exposes its `spec.dialects` gate) so a caller can count and report
  skipped, not-applicable rules instead of an engine's `scythe audit` reading as a clean pass when
  most rules never ran. (#167)
- `[lint.sqruff] enabled` was declared and never read: `enabled = false` did not disable sqruff.
  It now does. Separately, `[lint.sqruff.rules]` wrote any non-`"off"` value into sqruff's `rules`
  key, which sqruff treats as an *allowlist* — so `"LT02" = "warn"` silently disabled every other
  sqruff rule, the opposite of what it reads like and of what the docs claimed. sqruff has no
  per-rule severity at all, so only `"off"` can be honoured and any other value is now rejected with
  a message naming the offending key. An unknown rule code, previously swallowed, is also reported.
  (#113, #114)
- A rejected `[lint.sqruff]` table aborted the entire lint run and discarded every scythe-native
  finding along with it, because the sqruff call sits ahead of the rule engine in the per-file loop.
  A single typo could silently switch off the security rules, `SC-SEC07` PII detection included. The
  configuration is now validated once per `[[sql]]` block before any query file is read, so a config
  mistake is reported as one and the blast radius is visible rather than silent. Note validation
  lints a trivial statement: sqruff checks rule codes when a string is linted, not when the linter is
  built.
- `SqruffConfig::default()` returned `enabled: false`, the opposite of an absent `[lint.sqruff]`
  table, because `#[serde(default)]` only applies to absent TOML input and not to a derived `Default`.
  No call site hit it, but `enabled` only recently became load-bearing.
- Ruby `.rbs` signatures were emitted from a hardcoded scalar table rather than the backend manifest,
  so they could disagree with the `.rb` code generated beside them. `ruby-oci8` declared
  `created_at: Date` while the query bound a `Time`. Every RBS scalar now comes from the manifest.
  Regenerate to pick this up. (#106)
- A `[[sql.gen]]` entry missing its required `output` key produced a generic untagged-enum
  deserialization error that named neither the field nor the block. The error now names both, and
  unknown keys in a `[[sql]]` block are rejected rather than silently ignored. (#116)
- Every PHP manifest declared its `array` container as `array<{T}>`, which reached the generated file
  in a native type position where PHP has no generics: `public array<string> $tags` is a parse error.
  Two routes hit it — a PostgreSQL array column, and an `= ANY(...)` parameter, which the analyzer
  synthesises as `array<T>` in *every* dialect, so the broken type landed in function signatures even
  on engines with no array type of their own. (#200)
- All five JVM backends resolved a column's reader from a table maintained in parallel with the
  manifest, and every type outside that table fell through to an untyped accessor — `rs.getObject(col)`
  on the JDBC family, `row.get(col, Object.class)` / `Any::class.java` on the R2DBC pair. The declared
  field type came from the manifest, nothing compared the two, and the result did not compile:
  `incompatible types: Object cannot be converted to WidgetAddress`. Readers now derive from the
  declared type itself, so the two cannot drift. Three defects fell out of the same tables: the R2DBC
  arms matched `LocalDate` before `LocalDateTime` and read every datetime column as a date;
  `kotlin-exposed` called `wasNull()` nowhere, so a SQL NULL in a nullable `Int?` arrived as `0`; and
  all three JDBC backends read a nullable enum as `valueOf(getString(col).toUpperCase())`, an NPE on
  exactly the value the column exists to hold. (#191, #192, #213, #214)
- `java-r2dbc` emitted top-level records, a top-level enum and bare static methods into one
  compilation unit, and closed its `:grouped` row buffer with `});` while `.flatMap(` was still open.
  Neither had ever compiled. (#191)
- TypeScript emitted raw column names into positions that require an identifier. `ts_property_key`
  existed and was correct but only the row-struct emitters used it, so batch-params interface members,
  per-item binds, dot-access row reads and oracledb object-literal keys spliced the name verbatim —
  `first name: string;`, `[item.first name]`, and `row['it's']` closing its own quote. Property
  positions are now quoted; a scalar *parameter* named after a non-identifier column is still broken,
  because quoting is not available in a binding position, and is pinned by a failing-when-fixed test.
  (#215)
- `row_type = "zod"` derived its types from a table maintained beside the manifest, so the two
  disagreed on four of six columns in the same query: `active` was `number` under `interface` and
  `boolean` under `z.infer`, `price` `number` vs `string`, `created_at` `string` vs `Date`. Zod types
  now derive from the resolved TypeScript type, so `z.infer` equals the manifest type by construction.
  Enum variants also went through raw `to_pascal_case` and had their values spliced unescaped —
  `In-active: "in-active",` is not a valid key. (#216)
- `typescript-duckdb` imported `Connection`, which `@duckdb/node-api` does not export, and called
  `stmt.run(args)`, which takes no arguments. Every file this backend has ever produced failed to
  compile. Values now bind through `stmt.bind()`. (#217)
- `typescript-oracledb` bound the driver result to `const result` inside the block where the grouped
  fold declares its own (`Cannot redeclare block-scoped variable`), uppercased row keys
  unconditionally so a quoted lower-case column read as `undefined` with no compile error, and ignored
  `row_type = "zod"` entirely — there was a test certifying that no-op. (#218)
- The postgres.js `:batch` path rewrote `$N` with a raw string replace while every other command path
  used the literal-aware rewriter, so `VALUES ($1, $2, 'lit $1 $2 end')` turned an inert SQL string
  literal into two extra live bindings. It compiled, ran, and stored the wrong text. (#219)
- Six JSON functions were split across arms whose behaviour followed from which arm they landed in
  rather than from their semantics: `jsonb_agg` lost the nested-struct inference `json_agg` got,
  `to_json`/`to_jsonb` over a whole-row reference returned flat `json` and were hardcoded non-nullable
  despite being strict, and `json_strip_nulls` was likewise fixed non-nullable.
- The JSON function table is now derived from `pg_proc.proisstrict` and measured behaviour rather than
  assumption. Four functions had no arm at all, so legal PostgreSQL failed with a hard
  `unknown function` error: `array_to_json`, `json_object`/`jsonb_object`, `jsonb_set`/`jsonb_insert`
  and `jsonb_pretty`. `jsonb_set_lax` reports `proisstrict = f` but still returns NULL for a NULL
  target or path — only its replacement argument is exempt — so it gets its own arm. `json_typeof`,
  `jsonb_typeof` and `json_array_length`/`jsonb_array_length` were unconditionally nullable where the
  database is strict, and now follow their argument.
- A bare MySQL `?` placeholder used as a plain arithmetic operand in the SELECT list (`SELECT age + ?
  AS x FROM users`) was dropped entirely: `infer_expr_type`'s `Expr::Value` arm only resolved a
  placeholder's position via `parse_placeholder`, which parses `$N` but returns `None` for `?`, so the
  occurrence never reached `resolve_placeholder_position` and the generated function signature was
  missing an argument. Separately, `analyze_select` visited `WHERE`/`HAVING` before the projection, so
  a `?` textually first in the SELECT list was numbered *after* one appearing later in `WHERE` —
  `SELECT CAST(? AS CHAR) AS tag, name FROM users WHERE age = ?` bound the WHERE placeholder first.
  Projection is now analyzed before `WHERE`/`HAVING`, and the `Expr::Value` placeholder arm resolves
  through `resolve_placeholder_position` for both `$N` and `?`. (#170)
- **`java-r2dbc` and `kotlin-r2dbc` threw `IllegalArgumentException` on any null argument.** R2DBC's
  `Statement.bind(index, value)` rejects null outright — `bindNull(index, Class<?>)` is the only way
  to send SQL NULL — and both backends emitted `bind` for every parameter regardless of nullability,
  so a nullable parameter failed at the bind call rather than reaching the database. Ordinary
  nullable parameters now route through a generated `bindNullable` helper; a nullable enum gets an
  inline null check instead, because its bind expression calls `.getValue()`/`.value` on the field
  and would throw before any helper could test it. Both PostgreSQL harnesses gained a call that
  passes a real null, which is what makes the regression catchable: reverting the fix now fails them
  with the driver's own "value must not be null". The `Batch` bind sites are untouched and still have
  a separate pre-existing gap — a batch enum parameter binds the raw enum object with no
  `.getValue()`/`.value` call.
- **`java-r2dbc` and `kotlin-r2dbc`'s `:batch` bind sites never got either fix above.** They bound
  every parameter unconditionally (the same `IllegalArgumentException` on a null batch argument that
  ordinary bind sites had) and, for an enum, bound the raw Java/Kotlin enum object instead of its SQL
  spelling (the same "no codec for a user enum type" failure). Both backends' bind-site logic is now
  shared between the ordinary and `:batch` code paths through a `write_r2dbc_bind_for`/`r2dbc_bind_expr_for`
  pair that takes an explicit receiver expression (a loop variable or a batch-params record/data-class
  accessor) instead of always reading the parameter's own field. The PostgreSQL enum placeholder cast
  (`add_pg_enum_casts`) already reached `:batch` SQL before this fix, since it operates on the one `sql`
  local shared by every command shape — that part needed a test, not a fix. Covered by new backend unit
  tests only: the PostgreSQL fixture schema both harnesses build from (`integration_tests/sql/pg/queries/`)
  has no `:batch` query at all, so neither harness can yet exercise this path end to end — still an
  unfalsifiable gate at the integration level until a `:batch` fixture query exists.
- **A misspelled annotation (`@nullible`, `@optionall`, `@nonull`, ...) was captured and silently
  discarded.** Any `-- @<name> <value>` line scythe does not natively recognise is deliberately kept
  as an opaque `CustomAnnotation` — that escape hatch is how consumers layer their own annotation
  vocabulary (`@http`, `@http_auth`, ...) on top of scythe — but nothing ever inspected it, so a
  typo'd override behaved identically to one with no override at all while `scythe generate`,
  `scythe check` and `scythe lint` all reported success. `CustomAnnotation` now carries a
  `suggested_keyword` when the unrecognised name is within edit distance 2 of a known keyword
  (`name`, `returns`, `param`, `nullable`, `nonnull`, `json`, `deprecated`, `group_by`, `optional`),
  for a caller to turn into a warning. Left as a signal rather than a hard parse error: rejecting
  every unrecognised annotation would break the same consumer-defined vocabulary the escape hatch
  exists for. (#152)
- **`scythe migrate` reported every `sqlc.arg`/`sqlc.narg` name as "renamed" while discarding it.**
  It emitted `-- @param {name}`, which `scythe_core::parser` stores as a docs-only `ParamDoc` that
  the analyzer never reads for naming — only the positional `-- @param $N {name}` form becomes a
  `PositionalParamDoc` and actually renames the generated parameter. A migrated
  `sqlc.arg(needle)`/`sqlc.arg(mailbox)` query silently fell back to inferred or `pN` parameter
  names on the very next `scythe generate`, even though `migrate` printed "2 param(s) renamed".
  `migrate` now emits the positional form, with the same sequential numbering it already assigns to
  the placeholder. (#152)
- **Two SQL values of the same enum could collide on the generated variant name and `scythe generate`
  wrote both anyway.** `'gpt-3.5-turbo'` and `'gpt_3_5_turbo'` both sanitize and case-convert to
  `Gpt35Turbo` under `enum_variant_case = "PascalCase"` (Rust, C#, Go, TypeScript); nothing compared
  the rendered variant names before `generate_enum_defs_via_backend` handed them to a backend, so the
  file came out with `pub enum Model { Gpt35Turbo, Gpt35Turbo, }` — `E0428` under a real `rustc`, a
  redeclaration in every other target — while the command exited 0. A new `resolve::check_enum_variant_collisions`
  runs once per enum, the variant counterpart of the existing enum/query-type-name check, and rejects
  the query with `DUPLICATE_ALIAS` before any backend renders it. (#136)

- **Four more PostgreSQL manifests lost a nested aggregate's list-ness on degrade, the same way
  `java-jdbc.toml`'s `json = "String"` collapses `json_agg` down to one opaque string.**
  `elixir-ecto`, `php-amphp`, `typescript-kysely` and `typescript-postgres` now declare `json_array`
  for an array-shaped `json_agg`/`row_to_json` result their backend does not construct into a typed
  struct, each verified against the exact decode path an already-declared sibling manifest relies on:
  `elixir-postgrex`'s Postgrex/Jason pipeline for `elixir-ecto` (both run raw SQL through the same
  Postgrex binary protocol, verified live against PostgreSQL 16); `php-pdo`'s generated
  `json_decode($value, true)` for `php-amphp` (the same call, independently emitted); `typescript-pg`'s
  `pg` auto-parsing for `typescript-kysely`'s PostgreSQL dialect (documented as running over `pg`
  unchanged) and for `typescript-postgres`'s `postgres.js` driver (which parses `json`/`jsonb` the same
  way). Scope, precisely: `catalog_has_nested_aggregates` only infers a nested aggregate for the
  PostgreSQL dialect on a postgresql-family engine — Redshift and DuckDB are excluded by name — so only
  the 19 postgresql-engine manifests can reach this path at all, and after this change 4 of them build a
  real struct, 8 keep the array shape, and 7 still collapse to plain `json` (`csharp-npgsql`,
  `java-jdbc`, `java-r2dbc`, `kotlin-exposed`, `kotlin-jdbc`, `kotlin-r2dbc`, `ruby-pg`). Those 7 map
  `json` to a raw string with no driver- or codegen-level array decoding to point to, so a distinct
  `json_array` marker would carry no real information over `json` itself; `ruby-pg` is the one worth
  revisiting, since the `pg` gem can decode JSON but nothing in the generated code configures it. `json_nested` (a typed struct, not just array-shape) requires a
  backend-side decoder — `generate_nested_struct_def` — that a manifest alone cannot add, so it stays at
  its existing four (`rust-sqlx`, `rust-tokio-postgres`, `go-pgx`, `python-psycopg3`). (#147)

- **A `json_agg`/`row_to_json` column degraded to plain `json` (or `json_array`) on the 15 of 19
  PostgreSQL backends that do not implement `generate_nested_struct_def`, and nothing said so.**
  `degrade_unsupported_nested_structs` rewrote the column's neutral type and `scythe generate` exited 0,
  so a user asking for a structured nested row from, say, `java-jdbc` (`json = "String"`, read back via
  `rs.getString`) got an opaque string with no indication a struct was ever requested. The function now
  also returns one `NestedStructDegradation` per rewritten column — the SQL column name, the struct that
  could not be built, the fallback type it got instead, and the backend — threaded onto
  `GeneratedCode::degraded_nested_structs`. This is a library-side signal only: `scythe-cli` still needs
  to turn each entry into a reported finding (`scythe-codegen` cannot install a subscriber or depend on
  `scythe-cli`/`scythe-lint`). Not a hard error by default — failing every degrading backend outright
  would break working setups. (#147)

- **A bare `?` placeholder or literal `NULL` projected with no `CAST`/comparison/`COALESCE` to borrow a
  type from reached `analyze()`'s `Ok` result typed `neutral_type: "unknown"`, then surfaced two layers
  down as the backend's `INTERNAL_ERROR: unknown neutral type: unknown`** — the part of #170 the
  counting/ordering fix (c288fce1) left open. `analyze()` now rejects the shape with `TYPE_MISMATCH`,
  naming the query and column and suggesting an explicit `CAST`. The rejection is origin-based, not a
  blanket check on `neutral_type == "unknown"`: it only fires on a column whose projected expression is
  itself a bare placeholder/`NULL`, so a UNION arm's `NULL` that a sibling arm resolves, and a
  `jsonb_each`/`json_each` record column (legitimately `"unknown"` — PostgreSQL's `record` pseudo-type
  has no neutral-type representation), are both unaffected. (#170)

- **A UNION with both arms projecting a bare `NULL`, or a bare `NULL`/placeholder projected out of a
  derived table or CTE, still reached codegen as `INTERNAL_ERROR: unknown neutral type: unknown`
  instead of the clean `TYPE_MISMATCH` above.** Two places stripped the `untyped_literal` taint before
  `analyze()`'s final check ever saw it: a UNION arm's widened column was rebuilt with
  `..Default::default()`, always dropping to `false` regardless of whether either side actually
  resolved a type; and a derived table's or CTE's output columns were folded back into scope through a
  constructor that hardcoded `untyped_literal: false`, resetting the flag the moment a column crossed a
  subquery boundary. The UNION case now survives the taint only when *both* arms are untyped — either
  side supplying a real type still clears it, so a `NULL` arm a sibling resolves is unaffected — and
  `ScopeColumn` now carries the flag from an already-analyzed output column through a derived-table/CTE
  boundary via a new `from_analyzed_column` constructor, while a genuine catalog column or a function's
  synthetic result (`jsonb_each` included) is untouched and stays untainted no matter how many subquery
  or UNION layers it passes through. (#170)

- **The Java and Kotlin integration-test generators had no way to notice their per-engine harness
  branches drifting apart.** `java.java.jinja` and `kotlin.kt.jinja` duplicate a whole test program
  per SQL engine, and nothing compared what one branch tests against another — the redshift branches
  quietly ended up with fewer than half the postgresql branch's test functions. A new parity gate
  (`tests/engine_test_parity.rs`) now fails the build when an engine branch is missing a test
  function the postgresql branch of the same template has, unless it is named in the new
  `test-parity-exemptions.txt` ratcheting allowlist with a reason; the allowlist also fails on stale
  entries, so it can only shrink. Separately, `oracle/schema_full.sql` and
  `redshift/schema_pg_compat.sql` are runtime-only schema variants that harness templates apply
  independently of the (possibly different) file `scythe.toml` generated queries from — safe only as
  long as both files agree on table/column shape, which nothing checked; a new
  `tests/schema_variant_consistency.rs` now checks it. (#196, #195)

- **Two queries in one `[[sql]]` block whose `@name` values differed only in case could render the
  same function name into one file, and `generate` exited 0.** `CreateAPIKey` and `CreateApiKey` both
  `snake_case` to `create_api_key`; `check_file_level_type_name_collisions` already caught this shape
  for row/model structs and enums but never compared query function names, and `assemble_body` has no
  dedup pass at all for `query_fn` (unlike the struct/enum lists, it pushes every result's function
  unconditionally), so the collision always reached the output file as two function definitions. The
  check now also compares `fn_name` across every query destined for the same file. (#136)

- **`scythe generate` silently produced different bytes for the same input depending on whether
  `rustfmt` happened to be on `PATH`, and said nothing either way.** `format_rust_code_if_possible`
  piped `rustfmt`'s `stderr` to `/dev/null` and fell back to the unformatted code on a failed spawn,
  a non-zero exit, or a broken pipe, all indistinguishably. A missing `rustfmt` is now reported as a
  warning (and still never fails the run — a missing toolchain says nothing about whether the
  generated code is correct); `rustfmt` spawning and then rejecting the input is reported with its
  own stderr and, since Rust has no other tool-based validator, now counts as a `--validate-output`
  finding the same way every other backend's real-compiler check already does. (#167)
- **A misspelled annotation still reported success — nothing consumed the `suggested_keyword` signal
  #152 added to `CustomAnnotation`.** A typo like `@nullible` parsed clean, analyzed clean, and
  `scythe generate`/`check`/`lint` all exited 0 while the nullability override it named silently
  never took effect. New rule `SC-PARSE03` (`misspelled-annotation`) fires when
  `suggested_keyword` is set, naming both the annotation as written and the suggested keyword (e.g.
  `@nullible` → did you mean `@nullable`?). `Warn` by default, not `Error`: the same escape hatch
  the signal rides on is a deliberate extension point with legitimate shipping usage (`@http`,
  `@http_auth`), and `suggested_keyword` is a heuristic, not proof the annotation is wrong. Lives in
  `default_registry` rather than `parse_registry` (unlike `SC-PARSE01`/`SC-PARSE02`): it has a real,
  already-analyzed `LintContext` to inspect, so it needs no additional `scythe check` wiring beyond
  registration. The default registry now holds 59 built-in rules, up from 58. (#152, #167)
- **`scythe migrate` parsed sqlc's top-level `plugins:` array and each `gen.<lang>.package` field
  and discarded both with no diagnostic.** Neither has a `scythe.toml` equivalent — scythe has no
  wasm/process plugin system to receive `plugins:`, and no backend supports overriding the
  generated-code package/module name (every scythe Go file hardcodes `package queries`) — so there
  is no config key `migrate` could fill in for either. Both are now a `warning:` on stderr naming
  what was dropped, rather than silence. Left as warnings, not `invalid_config` errors like an
  unsupported `gen.<lang>` target: a hard error on the mere presence of `plugins:` would fail
  ordinary, fully-convertible v2 configs that declare it only to satisfy sqlc's own plugin
  resolution alongside an otherwise-unremarkable `gen.go` block. (#152)
- **`scythe-backend`'s type tests ran against a stale private copy of the manifests, not what
  scythe actually ships.** `crates/scythe-backend/test-manifests/{rust-sqlx,rust-tokio-postgres}.toml`
  had drifted from `crates/scythe-codegen/manifests/`: the private copies were missing
  `json_nested`, `sanitize_field_names` and the ~50-entry `reserved` keyword list entirely, and the
  tokio-postgres copy declared `range = "String"` where the shipped manifest declares
  `PgRange<{T}>`. Worse, a test asserted that stale `"String"` value directly, so it wasn't merely
  blind to drift — it actively pinned the bug, and would have broken the moment someone pointed it
  at the real file. `4ef83676` had already proven `PgRange<{T}>` correct by compiling the emitted
  wrapper with `rustc`. The private copies are deleted; both `types.rs` and `manifest.rs` tests now
  `include_str!` the manifests `scythe-codegen` ships, and assertions cover `reserved`,
  `sanitize_field_names`, `json_nested` and the corrected `range` value. (#157)

- **`scythe-conformance` never checked that a fixture's `expected.query.columns` matched what the
  analyzer actually produced for `query_sql`.** The four nullability assertions only ever iterate
  `analyzed.columns`, so a declared column absent from that list — a rename, a dropped `SELECT`
  item, a typo — was never fed into any of them: not a failure, not a skip, just silently never
  examined, even though every row still named it. `crate::runner::evaluate_fixture` now rejects a
  declared column the analyzer produced no match for, via a new
  `RunnerError::DeclaredColumnNotAnalyzed`. (#160)

- **A typo in a live-fixture's `live` block, a run, a row expectation, or an `engine_expectations`
  entry parsed clean and silently dropped the whole thing.** `LiveBlock`, `Run`, `RowExpectation`
  and `EngineExpectation` had no `#[serde(deny_unknown_fields)]`, and `null_together` /
  `engine_expectations` are `#[serde(default)]`, so a misspelled key evaporated instead of failing
  to parse. All four now reject unknown fields. (#160)

- **`DIVERGENCES.toml`'s `engine` field was a bare, unvalidated `String`.** A typo'd engine name
  (e.g. `"postgres"` instead of `"postgresql"`) loaded successfully and then matched no `Verdict`,
  forever, with no diagnostic — the entry would sit in the registry looking active while
  suppressing nothing. `DivergenceEntry::engine` is now typed as `Engine`, so an unrecognized
  engine name fails to deserialize instead. (#160)

- **The three unit tests in `scythe-conformance/src/executors/mssql.rs` ran in no CI job at all.**
  `ci.yml`'s `test` job runs `cargo test --workspace` with this crate's default (empty) feature
  set, which never compiles the `mssql`-gated module; `nullability-conformance.yml`'s mssql job
  passes `--test live`, which restricts `cargo test` to that one integration binary and excludes
  lib unit tests. `ci.yml` now runs `cargo test -p scythe-conformance --features mssql --lib` as
  its own step, on every push and pull request. (#160)

### Changed

- **Six of the surviving `range` mappings named a type their driver does not produce, and are
  corrected; a seventh is removed.** Verified by running each real client against a live PostgreSQL
  instance rather than by reading the manifests. `csharp-npgsql` said `string`, but `GetString`
  throws `InvalidCastException` — now `NpgsqlTypes.NpgsqlRange<{T}>`. `go-pgx` said `string`, but
  pgx v5 refuses to scan a range into `*string` at all — now `pgtype.Range[{T}]`. Both Elixir
  manifests said `String.t()` where Postgrex returns `%Postgrex.Range{}` and rejects a plain string
  as a bind parameter. Both Python manifests said `tuple[{T}, {T}]`, and neither driver returns a
  tuple; they now name `asyncpg.Range[{T}]` and `psycopg.types.range.Range[{T}]` respectively,
  which are genuinely different classes, so the family spelling legitimately diverges.
  `rust-tokio-postgres` has no usable mapping — `postgres-types` excludes every range OID from
  `FromSql for String` and ships no range decoder — so its declaration is removed and recorded as a
  capability exception that fails the gate if anyone re-adds one without justification. Any query
  selecting a range column on these backends generated code that did not work; it now does, but the
  host type it names has changed. (#190)

- **The `range` container is now declared only on PostgreSQL manifests — 84 declarations become 19.**
  `range` was mapped in 84 of 102 manifests in seven mutually incompatible spellings, and nothing
  asserted anything about it. Presence tracked no engine capability in either direction: it was
  declared for MySQL, MariaDB, SQLite, MSSQL, Oracle, DuckDB and Redshift, none of which has a
  PostgreSQL-style range column type, and omitted from manifests whose siblings declared it. The
  spellings contradicted each other inside single language families and, in two cases, inside one
  file — `python-asyncpg` said `tuple[{T}, {T}]` while `python-asyncpg.redshift` said `str`;
  `elixir-postgrex` said `string()`, which is not the Elixir typespec for a binary at all. Dropping
  the key on an engine with no range type is a degradation only in the sense that a query can no
  longer silently resolve `range<T>` to a wrong host type there — it now falls through the unknown-
  container path like any other unmapped container. `range_container_consistency.rs` gates presence
  against engine and spelling against each family's own `string` scalar, in both directions. (#190)
- **`scythe fmt --check` exits 2 rather than 1 when files need formatting.** #212 reserves exit 1 for
  operational failure — an unreadable file, an invalid config — and a distinct code for "the thing
  you asked about is not satisfied", which is what `lint` and `check` already do. `fmt --check` used
  a plain error for both, so a CI step could not tell a formatting difference from a broken run.
  Scripts branching on exit 1 from `fmt --check` need updating.
- **Breaking (`elixir-ecto`): the backend emits Ecto instead of a Postgrex clone.** Generated
  functions take a `repo` rather than a `conn`, specs change from `Postgrex.conn()` to
  `Ecto.Repo.t()`, queries run through `Ecto.Adapters.SQL.query(repo, sql, args, [])`, and `:batch`
  uses `repo.transaction/1` with `repo.rollback/1`. Struct definitions also move to top level instead
  of nesting under `Scythe.Queries.*`. A backend named after Ecto that generated raw Postgrex calls
  was misnamed rather than merely limited. Every caller of a generated `elixir-ecto` function must
  now pass a repo module. (#202)
- The PHP backends now render a type twice: the native position (property, parameter, return) keeps
  the bare `array` PHP's syntax requires, and `@var`/`@param` docblocks get `array<T>` back. A bare
  `array` is `array<mixed, mixed>` to PHPStan, so the fix that made the output parse cost every array
  column and every `= ANY(...)` parameter its element type at level 9. Manifests gained an optional
  `[types.docblock_containers]` table, which falls back per container name to `[types.containers]`;
  only the nine `php-*.toml` manifests declare it, and every other language's output is byte-identical.
  Measured on the torture schema at PHPStan level 9: 15 findings to 9 (php-pdo), 24 to 18 (php-amphp),
  all six removed being `missingType.iterableValue`.
- Dependency pins for `integration_tests/**` are managed by Renovate against the jinja templates in
  `tools/integration-test-generator/templates/`, which is where they actually live. Dependabot cannot
  target generated files, so its PRs against them were dead on arrival. (#115)
- `snowflake-jdbc` is unified on 4.0.2 across both JVM templates, which previously disagreed.
- **The JVM backends have a real array reader, and array columns are `List<T>` again.** An earlier
  revision of this release degraded the JVM manifests to declare array columns as `String`, because
  no JVM backend could read one and `int[]`/`bool[]` additionally rendered `java.util.List<int>`,
  which is not valid Java. That entry said the degradation was not the destination; the reader now
  exists, so `array` maps to a boxed `List<T>` and the `String` fallback is gone. Regenerated JVM
  output changes shape for every array column. (#192)
- `scythe lint` and `scythe fmt` build one sqruff linter per `[[sql]]` block instead of one per file.
  Construction compiles the dialect's lexer, so a run over N files paid N+1 constructions where one
  suffices; measured on 200 single-query files in one block, `scythe lint` goes from 0.547s to 0.047s.
  Construction is also validation, which moves an invalid `[lint.sqruff]` table from "error against
  whichever file was read first" to an error about the config itself. (#130)

### Removed

- **Breaking (`scythe-codegen`)**: removed the public `generate_from_catalog` stub. It ignored its
  argument and always returned `Ok(GeneratedCode::default())`, so a caller could not distinguish
  "nothing to generate" from "this function does nothing" — reporting success while doing nothing is
  worse than not existing. It had no caller besides its own tautological test, which asserted the
  stub's behavior matched the stub's behavior and could never fail. If catalog-level codegen is
  implemented later it should land as a real implementation, not a reserved name. (#132)
- **Breaking (`scythe-backend`)**: removed `BackendRenderer`, its jinja fixtures, and the
  `BackendError::TemplateError` variant, along with the crate's `minijinja` dependency. Code
  generation is done by the per-language emitters in `scythe-codegen`; the template renderer was a
  parallel mechanism with no production caller, so it read as a supported extension point that did
  not exist. Breaking only for a caller matching directly on that error variant.
- Four `r2dbc` manifests for engines the r2dbc backends never accepted. They were unreachable.
- **Breaking (`scythe-lint`)**: removed the free functions `sqruff_adapter::validate_config`,
  `lint_sql`, `lint_and_fix_sql` and `format_sql`. Each built a `SqruffLinter` per call, and building
  one compiles the dialect's lexer — the cost that dominated `scythe lint` until #130 hoisted
  construction out of the per-file loop. Leaving them in left a second way to do the same thing where
  the obvious use (a loop over files) silently reintroduced that cost. Use `SqruffLinter::for_linting`
  (returns `None` for `[lint.sqruff] enabled = false`) with `lint` / `lint_and_fix`, or
  `SqruffLinter::new` with `format`, building one linter per run instead of per file;
  `validate_config` is `for_linting` with the linter discarded, so keep the linter. None were
  re-exported from the crate root, so only a caller naming `scythe_lint::sqruff_adapter::` directly is
  affected. (#130)

## [0.14.0] - 2026-08-09

This release checks scythe's output against something other than scythe. Nullability inference is
measured against what live database engines actually return across all six of them. Generated files
now record the schema they came from, and `scythe check` can diff your DDL against the database the
code will actually run against. A Snowflake backend that had never executed anywhere runs in CI for
the first time.

Three of those checks found real bugs the model-only tests could not: Oracle returns NULL for an
empty string literal, `typescript-duckdb` had been reading rows positionally while indexing them by
name, and `typescript-oracledb` cast nullable columns to their non-null type. All three are fixed
below.

**Upgrading**: `scythe check` now exits `2` for findings and `1` for operational failures, so a CI
script keying on `1` needs updating. Unrecognized options on any `[[sql.gen]]` target — not just
TypeScript — are now an error rather than silently ignored. Identifiers derived from names with
consecutive capitals change spelling (`CreateAPIKeyRow` becomes `CreateApiKeyRow`). And every
generated file gains a provenance header, so the first regeneration after upgrading touches every
artifact. See **Changed** for the full list.

### Added

- A per-target manifest override: `manifest = "..."` on a `[[sql.gen]]` target names a **partial**
  manifest merged over the backend's compiled-in one, so a project can retarget a few type mappings,
  naming fields or import rules without vendoring a whole manifest. The path resolves against the
  directory containing `scythe.toml` — the same rule every other path in the config follows since
  0.13.0 — so generated output does not depend on where the command was run. The override is keyed
  per target rather than per backend name: `rust-sqlx` covers five engines and `java-jdbc` nine,
  each with its own type mappings, and a `[[sql.gen]]` target inherits its engine from the enclosing
  `[[sql]]` block, so two targets naming the same backend under different engines each get their own
  file. `[types.scalars]` and `[types.containers]` may only replace mappings the backend already
  defines — neutral type names are a fixed vocabulary, so a key outside it is a typo, and the error
  suggests the near miss; `[imports.rules]` does accept new keys, because retargeting a scalar
  requires an import rule keyed on the new type's prefix. There is no `[backend]` section: manifest
  selection stays a pure function of `(backend, engine)`. Every failure — unknown section, unknown
  key, missing file — fails `scythe generate` naming the backend, the resolved absolute path and the
  offending key, and nothing falls back to the compiled-in manifest silently
  ([#82](https://github.com/Goldziher/scythe/issues/82))
- A live nullability conformance suite (`scythe-conformance`), a dev-only workspace member that
  compares inferred nullability against what engines actually return rather than against scythe's
  own model. Per (fixture, engine, column) it holds three facts side by side: the analyzer's
  verdict, whether the generated code actually *renders* the column as optional (parsed out of the
  resolved type against the backend manifest, deliberately not copied from the analyzer, so the two
  can genuinely disagree), and the engine's observed per-row nullness from a real query run. Four
  assertions relate them: fidelity (analyzer and generated code agree), soundness (an observed NULL
  implies the generated code renders the column optional), anti-vacuity (a column called nullable
  must be demonstrated NULL by some run, or the suite is satisfied by marking everything nullable),
  and join-group coherence (columns widened by the same outer join go NULL together). Accepted
  over-pessimism — the analyzer is stricter than an engine turns out to be — goes in a capped
  registry with a tracking issue per entry, and an entry that stops reproducing fails the build, so
  fixing the gap forces deleting the entry that excused it. A soundness failure is never
  suppressible by any registry entry. The crate is unpublished, has no CLI surface, and nothing in
  `scythe generate` touches it
- Live drivers in that suite for all six engines — PostgreSQL, MySQL, MariaDB, SQLite, SQL Server and
  Oracle — each behind its own Cargo feature plus a `live-tests` gate, with one CI job per engine. No
  driver is linked by default, so `cargo test --workspace` exercises the pure comparison logic
  without a container. Selecting an engine whose feature was not compiled in is a hard error naming
  the feature to enable, never a silent skip. Each engine's isolation is its own problem: SQL Server
  gets a fresh database per connection, because T-SQL's default schema belongs to the database
  principal rather than the session and `USE` does not survive tiberius routing statements through
  `sp_executesql`, so tables would otherwise land in `master`; Oracle gets a user per connection,
  because in Oracle a schema is a user. Fixtures are now analyzed under their own engine's dialect
  rather than PostgreSQL's, which is what made the Oracle empty-string bug below observable
  ([#71](https://github.com/Goldziher/scythe/issues/71))
- **A provenance header in every generated file**, recording the scythe version, backend, engine, a
  fingerprint of the schema and a fingerprint of the query set the file was generated from:

  ```text
  // scythe:provenance v=0.14.0 backend=go-pgx engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:9c4e1f77a0b3d582
  ```

  The comment token follows the target language (`#` for Python, Ruby and Elixir), and where a
  language requires particular first bytes the header goes second — after `<?php`, after Ruby's
  `# frozen_string_literal: true`. Python's variant carries a trailing `# noqa: E501`, since the line
  exceeds ruff's default 88-column limit and would otherwise fail `ruff check` on line 1 of every
  generated Python file. Ruby's `queries.rbs` signature file gets one too.

  The schema fingerprint is a SHA-256 over a canonical rendering of the *resolved* catalog — tables,
  columns, enums, composites, domains, dialect — rather than over DDL text, so reformatting a
  migration or reordering statements does not move it while a real schema change does. It excludes
  column defaults (free-form AST text that churns on dependency bumps) and scythe's own version,
  which would otherwise report every artifact as drifted on every release.

  The query fingerprint covers the *analyzed* query set — each query's name, command, parameter names
  and types, and resolved column names, types and nullability — rather than the SQL text, so
  reformatting a `.sql` file or editing a comment does not report drift while a change that moves a
  generated signature does. Parameter names participate because every backend emits them as the
  generated function's argument names: swapping `WHERE name = $1` for `WHERE email = $1` leaves both
  parameters typed `string` but rewrites the signature from `name: &str` to `email: &str`, breaking
  every caller. Schema drift and query drift are reported as distinct findings, `SC-PRV01` and
  `SC-PRV08`.

  Generated code is only as correct as the schema snapshot scythe read, and nothing in the artifact
  recorded which snapshot that was. Code generated against a drifted local schema compiles, reviews
  as an ordinary diff, and meets a different migration state in production. Raised by **Mads
  Hansen**.

  `scythe check` verifies it through seven rules: `SC-PRV01` schema drift, `SC-PRV03` backend drift
  and `SC-PRV04` engine drift as errors; `SC-PRV02` scythe-version drift, `SC-PRV05` missing header,
  `SC-PRV06` malformed header and `SC-PRV07` unverifiable header as warnings. They are ordinary
  registry rules, so `[lint.rules]` and `[lint.categories]` can downgrade or disable any of them. A
  scythe upgrade alone is a warning by design — upgrading the tool must never fail a consumer's CI
  before they have had a chance to regenerate. A missing artifact produces no finding at all, since
  the default `.gitignore` excludes `**/generated/`.

  It answers "was this generated from this schema?", not "from these queries?" — editing a query file
  without touching the schema produces no mismatch
  ([#68](https://github.com/Goldziher/scythe/issues/68))
- **`scythe check --database-url` now also diffs your DDL against the live database.** PostgreSQL
  only; blocks on other engines are skipped with a warning naming the block. Seven `SC-DRF` rules
  cover tables and columns missing from either side, type mismatches, nullability mismatches and enum
  value mismatches — all errors except `SC-DRF02`, a table present in the database but not in your
  DDL, which is a warning because every real database has a `schema_migrations` table scythe knows
  nothing about.

  `SC-DRF06` is the rule that justifies the feature. Query verification cannot check nullability:
  preparing a statement makes PostgreSQL report type OIDs and nothing about NULL-ness. Reading
  `pg_attribute.attnotnull` is the only way scythe can tell you a `NOT NULL` in your DDL is not true
  in production, where the generated non-optional field fails to decode the first NULL row it meets.

  The catalog is read from `pg_catalog`, not `information_schema`, which reports `USER-DEFINED` for
  every enum column and never names the type — the enum and type-mismatch rules would be undetectable
  through it. Type comparison is exact equality rather than the tolerant predicate query verification
  uses, which forgives string widening and so reported nothing when a `text` column became `uuid`.
  Types neither side can express in scythe's neutral vocabulary are skipped rather than reported as
  mismatches. Views and materialized views are excluded from the nullability check, since PostgreSQL
  stores `attnotnull = false` for every view column.

  Opt-in via the flag: with no `--database-url` nothing connects, and unlike `scythe inspect`, `check`
  never falls back to `$DATABASE_URL` — it cannot start requiring a database because that variable
  happens to be set, which is what keeps it usable in a pre-commit hook.

  This is the cheaper intermediate the issue proposes, not its headline `schema_source = "execute"`
  design: scythe still builds its catalog by parsing DDL and does not execute your migrations against
  an ephemeral database, so DDL that only a real engine can resolve is still out of reach. Raised by
  **u/Character-Forever-91** ([#79](https://github.com/Goldziher/scythe/issues/79))
- **Nested struct types inferred from `json_agg(alias.*)` and `row_to_json(alias.*)`.** A column that
  aggregates a whole relation now resolves to a generated struct with that relation's fields instead
  of an opaque JSON scalar. PostgreSQL and CockroachDB only, and only on `rust-sqlx`,
  `rust-tokio-postgres`, `go-pgx` and `python-psycopg3` — every other backend degrades to exactly the
  plain `json` mapping it produced before, including on Redshift, where `json_agg` does not exist.

  Element nullability is modelled on the element, not the fields. `json_agg` over a LEFT JOIN emits a
  JSON `null` element, never an object of null fields, so an inner join yields
  `Vec<GetUserOrdersRowOrders>` and an outer join `Vec<Option<GetUserOrdersOuterRowOrders>>`.
  Widening the fields instead would model a value PostgreSQL never produces while leaving the type
  unable to hold the one it does.

  JSON keys are the raw SQL column names, so a quoted `"createdAt"` column gets `#[serde(rename)]` in
  Rust, a `json:"createdAt"` struct tag in Go, and an explicit `_from_json` classmethod in Python —
  `Cls(**item)` would pass `createdAt` as an unexpected keyword argument. Enums reachable only
  through a nested struct are emitted with per-variant renames, since the driver's own
  `rename_all` tells serde nothing.

  `jsonb_agg` is deliberately not covered, nor is `json_agg` over a scalar or a bare `*`. Two queries
  in one file deriving the same struct name deduplicate if their shapes match and are a hard error
  naming both if they do not. Your own `@json` annotations are untouched
  ([#78](https://github.com/Goldziher/scythe/issues/78))
- **A `field_case` option** on a `[[sql.gen]]` target, accepting `snake_case` (the default) or
  `camelCase`, honored by the 11 TypeScript backends and by `java-jdbc`, `java-r2dbc`, `kotlin-jdbc`,
  `kotlin-r2dbc` and `kotlin-exposed`. It renames generated field and parameter names only; every
  backend still reads the driver row by the raw SQL column name, so the rename cannot break decoding.

  On TypeScript the naive version of this would ship a type that lies: 10 of the 11 backends return
  the driver's row through a blind `rows[0] as StructName` cast, so renaming the declared field alone
  type-checks green and returns `undefined` for every field at runtime — `tsc` certifies the bug.
  Under `camelCase` the function body therefore reconstructs the row field by field, reading raw keys
  and writing renamed ones ([#87](https://github.com/Goldziher/scythe/issues/87))
- **Four JSDoc `javascript-*` backends**: `javascript-pg`, `javascript-postgres`,
  `javascript-mysql2` and `javascript-better-sqlite3`. They emit plain ESM `.js` carrying its types
  entirely in JSDoc comments — `@typedef`/`@property` for row types, `@param`/`@returns` for
  functions — with no driver import statement, referencing driver types inline as
  `import("pg").PoolClient`. Nullability is always `T | null`, never the optional-property form.
  These are an emit mode on the existing TypeScript backends rather than new manifests, so the
  manifest count is unchanged. `row_type = "zod"`, `outer_join_unions` and `field_case = "camelCase"`
  are rejected with an error naming the TypeScript backend to use instead: each needs syntax a plain
  `.js` file cannot carry. Output is validated in CI with real `node --check` and
  `tsc --checkJs --strict` ([#81](https://github.com/Goldziher/scythe/issues/81))
- `--exit-zero` on `scythe check`, matching the flag `audit` and `inspect` already had
- **A `queries=` fingerprint in the provenance header**, covering the analyzed query set alongside
  the schema. The schema fingerprint said nothing about the `.sql` files, so editing a query and
  forgetting to regenerate left an artifact that `scythe check` called clean. Computed over each
  query's name, command, parameter names and types, and resolved column names, types and
  nullability — not the SQL text — so reformatting a query or editing a comment stays silent while a
  change that moves a generated signature does not. Parameter names participate because every
  backend emits them as the generated function's argument name: swapping `WHERE name = $1` for
  `WHERE email = $1` leaves both typed `string` but rewrites the signature from `name: &str` to
  `email: &str`, breaking every caller. Reported as `SC-PRV08` (`query-drift`, Error), distinct from
  `SC-PRV01`, so a drift report names which of the two moved. An artifact whose header predates this
  field is not reported as malformed ([#94](https://github.com/Goldziher/scythe/issues/94))
- **`ErrorCode::InvalidConfig`**, so a mistake in `scythe.toml` no longer surfaces under
  `INTERNAL_ERROR` and read as a scythe bug rather than something the user can fix. `ErrorCode` is
  now `#[non_exhaustive]`, so adding a future variant is no longer a breaking change ([#102](https://github.com/Goldziher/scythe/issues/102))
- Live nullability coverage for seven more rules: `MAX`/`AVG` over an empty set, `NULLIF` with equal
  operands, `CASE` with no `ELSE`, a scalar subquery matching no row, `RIGHT JOIN` and `FULL JOIN`.
  Each is asserted against real engines rather than against the analyzer's own model. `FULL JOIN`
  joins users to tags rather than users to orders, because the foreign key on `orders.user_id` means
  no order can ever be unmatched and the join would degenerate to a `LEFT JOIN` — proving half the
  rule while appearing to prove all of it. It is scoped away from MySQL and MariaDB, which parse
  `FULL` as a table alias and reject the query ([#71](https://github.com/Goldziher/scythe/issues/71))

### Changed

- **The README is a front page again, and its code is checked.** It was 412 lines, 227 of them ten
  hand-written samples of "generated" code that no tool validated — and five of the ten were wrong,
  including a Go block still showing pre-0.14.0 field casing and typing `NUMERIC` as `*string`. It
  now carries one Rust sample copied verbatim from committed generated output, and `README.md` is
  in `snippet-runner`'s reference set so CI compiles it. The feature list and the language/database
  matrix moved to the docs pages that own them.
- **The documentation was audited against the source and corrected.** Seven claims contradicted the
  implementation: the unknown-option rule was described as TypeScript-only; `[lint.sqruff] enabled`
  was documented as a working toggle when nothing reads it; `[lint.sqruff.rules]` was documented as
  the opposite of the allowlist it actually builds; `--dialect` on `lint`/`fmt` was described as
  unset; the crate table omitted `audit` and `inspect`; `scythe fmt` was described as running
  sqruff's full default rule set, when `LT01` is excluded there too; and
  `[sql.gen.python|typescript|go|kotlin]` were supported but undocumented. Per-language backend
  pages replace the combined Java/Kotlin page and the "Other" grab-bag, with the old URLs kept as
  stubs.
- **`llms.txt` generation no longer destroys the code samples.** `minify.collapseCodeBlocks` was
  enabled, which collapses whitespace inside code fences as well as prose — so the abridged file
  rendered the whole site as 258 lines with every sample unparseable. It is off; the changelog and
  Starlight's anchor markup (22% of the corpus between them) are excluded; and `llms.txt` now
  carries the facts a model gets wrong by default, above all that the annotation syntax is not
  sqlc's. Backends, databases and guide are addressable as separate sets.
- **Generated-code tool validation reports a skipped checker as a skip, not a pass.**
  `validate_with_tools` returned `Option<Vec<String>>`, where `None` meant "tool not installed" and
  every call site spelled `if let Some(errors)` — so a checker that was never installed was
  indistinguishable from one that ran and found nothing. In practice 31 of 76 backend tests were
  passing without any tool touching the generated code: 13 because `biome` was installed nowhere,
  and 18 because Java, C#, Elixir and Rust have no validator at all. `validate_python_tools` was the
  worst case, returning `Some([])` when `ruff` was absent. The return type is now `ToolValidation`,
  reporting each checker separately as `Ran`/`Missing`/`Failed`, and CI runs with
  `SCYTHE_VALIDATE_STRICT=1`, where a missing tool fails the build instead of being skipped
  ([#98](https://github.com/Goldziher/scythe/issues/98))
- **Generated code is checked by `poly`**, this repository's linter, rather than by a per-language
  collection of separately-installed binaries. poly bundles its engines in-process -- `oxc` for
  TypeScript, `ruff` for Python, `mago` for PHP -- so one already-required tool replaces `biome`,
  standalone `ruff`, the `python3 -m ast` syntax pass and `php -l`, and CI drops four install steps
  along with the Python, PHP and JDK toolchains it only needed in order to feed them. `node` and
  `tsc --checkJs --strict` remain for the `javascript-*` JSDoc backends, since oxc lints JavaScript
  but does not typecheck JSDoc annotations; `gofmt` and `ruby -c` remain because poly delegates those
  languages rather than bundling them. Kotlin loses its tool validation: poly delegates to `ktlint`,
  and standing up a JVM plus a downloaded jar to lint generated Kotlin is out of proportion to what
  it catches -- `validate_structural` still covers those backends, and an inventory test keeps the
  gap visible. Validation runs against a dedicated `generated-code-poly.toml` passed explicitly,
  because poly resolves config by walking up from the file it is handed and a temporary file finds
  nothing, so what CI enforced would otherwise depend on where the system temp directory sits
- **Unknown `[[sql.gen]]` keys are rejected on every backend**, not just TypeScript. 24 of the 52
  backends had no `apply_options` at all and inherited a permissive default that accepted anything
  ([#103](https://github.com/Goldziher/scythe/issues/103))

- **`scythe check` exits `2` for findings and `1` for operational failures.** It previously exited
  `1` for both, so a CI script could not distinguish "your schema drifted" from "your config file is
  missing". Any script or hook keying on `1` for findings must change to `2`, or use `--exit-zero`
  for an advisory gate. Warnings have never affected the exit code and still do not
- **Unrecognized options on any `[[sql.gen]]` target are now a hard error.** Every backend previously
  inherited the default `apply_options`, which silently discarded any key it did not read —
  `row_typ = "zod"` parsed as valid TOML and did nothing, with no diagnostic. This was fixed for the
  11 TypeScript backends first; the same typo behaving differently depending on target language was
  itself a trap, so the `CodegenBackend` trait default now rejects every key unless a backend
  declares it known, closing the gap for the other 41 backends in one change
  ([#103](https://github.com/Goldziher/scythe/issues/103)). Unknown keys fail generation, with a
  suggestion when the key is within edit distance 2 of a real one. A config carrying a
  forward-compatibility key on any target will fail on upgrade
- **Identifiers derived from names with consecutive capitals change spelling.** PascalCase conversion
  previously returned mixed-case input containing no underscore unchanged, so a query named
  `CreateAPIKey` generated the function `createApiKey` returning the type `CreateAPIKeyRow` — the
  same query, spelled two ways. Both sides now normalize identically: `CreateAPIKeyRow` becomes
  `CreateApiKeyRow`. This affects row, enum and composite type names on every backend, and function
  names on the 16 backends whose manifests use PascalCase function naming (all `csharp-*` and all
  `go-*`). Names that are snake_case, or PascalCase without consecutive capitals, are unaffected
- **Two SQL columns whose names collapse onto one field name are now a hard error.** `SELECT
  "USER_ID", user_id FROM t` is legal SQL and passes the analyzer's case-sensitive duplicate-alias
  check, which runs on raw names before conversion; it previously emitted a struct with two
  identically-named fields. This applies on every backend, including under the default `snake_case`,
  and covers parameters as well as columns
- **The `Auto-generated by scythe. Do not edit.` banner is gone**, replaced by the provenance header,
  which carries the same warning plus the version, backend, engine and schema fingerprint — and which
  `scythe check` verifies rather than merely stating. Removed from 40 of the 52 backends; the Go,
  Kotlin and Elixir backends never emitted it. Go's `// Code generated by scythe. DO NOT EDIT.` line
  is unchanged, since the Go toolchain matches on it. Together with the header, the first
  regeneration after upgrading touches every generated file

### Fixed

- **`[sql.gen.rust] serde` and `derive` failed on three of the four Rust backends.** Both keys are
  documented on the legacy table independently of `target`, and the CLI puts them into the options
  map whichever backend was selected — but only `rust-tokio-postgres` recognized them.
  `rust-sqlx` accepted `structs_only` alone, and `rust-tiberius` and `rust-sibyl` declared no
  options at all, so the reject-by-default trait default turned the documented config into
  `unknown option 'serde'`. All four now accept both keys. Emitted output is unchanged when neither
  is set.
- **`scythe audit --list-rules` under-reported the rule set by two.** It printed 56 rules against a
  registry of 58, and `--explain SC-A02` reported no such rule. Both built their catalog from the
  active-rule set, which drops anything resolving to `off` — correct for deciding what runs, wrong
  for a catalog. `SC-A02` and `SC-C01` are off by default but are registered rules a user can
  enable, and they now appear with their `off` severity. What actually fires is unchanged.
- **The schema fingerprint could report drift that was not real, and miss a change that was.**
  Schema-qualifier stripping was gated on PostgreSQL although `Catalog::get_table` is dialect-blind,
  and it stripped only `public.` where `get_table` strips any prefix, so `myschema.users` and
  `users` diverged. Enum values were emitted unescaped, so `ENUM ('a|b')` and `ENUM ('a','b')`
  produced an identical canonical line, and a value containing a tab or newline could forge an
  entire extra record. `CREATE DOMAIN` was absent from the canonical form. The escaping scheme
  escapes only when a delimiter is actually present, so every fingerprint in the corpus is unchanged
  and `FINGERPRINT_ALGORITHM_TAG` is deliberately not bumped — bumping it would hand every user a
  spurious drift report on upgrade. Eight fixture fingerprints are now pinned to their released
  values ([#91](https://github.com/Goldziher/scythe/issues/91))
- **`scythe migrate` wrote configs that could not generate.** A stock sqlc-for-Go config produced
  `backend = "go-go"`, since the language and target were concatenated unconditionally. Kotlin was
  worse: it had no field in the legacy config at all, so a migrated Kotlin project silently emitted
  `rust-sqlx` output. This is the on-ramp for every sqlc user ([#97](https://github.com/Goldziher/scythe/issues/97))
- **Generated Go failed to compile against a schema with no temporal or decimal column**, because
  the import block was emitted unconditionally across `go-pgx`, `go-godror` and `go-gosnowflake`.
  `gofmt -e` parses but does not typecheck, so it could never have caught an unused import
  ([#100](https://github.com/Goldziher/scythe/issues/100))
- **`ruby-sqlite3` and `ruby-tiny-tds` emitted an `.rbs` signature contradicting the `.rb` beside
  it**, because the RBS generator hardcoded the `json` type instead of following the manifest.
  Manifest scalars are Ruby *doc* names rather than RBS types, so this needed a translation table,
  not a passthrough ([#101](https://github.com/Goldziher/scythe/issues/101))
- **`WITH t(a, b) AS (...)` ignored the column alias list.** A CTE's explicit column names were never
  applied to the analyzed scope, so the outer query could not reference them: `WITH t(a, b) AS
  (SELECT 1, 2) SELECT a, b FROM t` failed with "column a does not exist", because a body projection
  carrying no names of its own labels its columns `unknown`. The alias list is now applied at all
  three registration points -- the recursive anchor's seed, the widened recursive union shape, and
  the plain fall-through path -- and an alias count that disagrees with the body's column count is
  rejected the way PostgreSQL rejects it, rather than being matched positionally into mislabelled
  columns. Contributed by **Znie** ([#107](https://github.com/Goldziher/scythe/pull/107))
- **`LEAD`/`LAG` were always inferred nullable**, even when both the argument and the three-argument
  default are non-null. `IGNORE NULLS` bails out, since it can return NULL regardless ([#89](https://github.com/Goldziher/scythe/issues/89))
- `typescript-kysely` output now carries a note recording that it requires `CamelCasePlugin`
  ([#96](https://github.com/Goldziher/scythe/issues/96))
- The grouped-fold row reads in `typescript-pg` and `typescript-mysql2` are cast in TypeScript mode,
  matching every other read site. The two `js_mode` sites stay uncast, since `as T` is not valid
  JavaScript ([#95](https://github.com/Goldziher/scythe/issues/95))

- **`csharp-snowflake` generated parameter bindings that Snowflake rejects.** Generated code named
  each positional binding `p1`, `p2`, `p3`, but Snowflake's REST protocol keys `?` placeholders by
  bare ordinal, so the server read them as *named* bindings and the query failed. Bindings are now
  named `"1"`, `"2"`, `"3"`. The backend had shipped since 0.6.0 without ever running anywhere: it
  was held out of the Snowflake integration job on the diagnosis that fakesnow's binding-name
  heuristic was at fault, and 0.12.0 recorded that as a fakesnow limitation rather than a codegen
  one. The diagnosis was backwards — fakesnow was right to reject `p1`, and real Snowflake would
  have rejected it too. The backend now runs against the shared fakesnow server in CI
- **`scythe migrate` silently converted nothing when the project directory contained a glob
  metacharacter.** The pattern used to find `.sql` query files was built by joining the base
  directory onto the `queries` entry, and the "is this already a glob, or a bare directory needing
  `/*.sql`" decision was then made on the *joined* string. Neither step escaped the base directory,
  so a project in a directory named e.g. `a[b]` had its `[b]` compiled as a glob character class,
  matched no files, and reported `Migration complete: 0 file(s) converted` instead of converting
  anything — the same silent-zero failure mode #84 fixed for `generate`/`check`/`lint`/`audit`/`fmt`
  in 0.13.0. The base directory is now escaped with `glob::Pattern::escape` and joined with `/` on
  every platform, and the directory-vs-glob decision is made on the raw pattern, never on the string
  after the base directory has been prefixed onto it
  ([#88](https://github.com/Goldziher/scythe/issues/88))
- `task version:check` asserted that every crate's own version matched `scythe-cli`'s, including
  crates marked `publish = false`. An unpublished crate never reaches crates.io, so its version
  carries no meaning and `version:sync` deliberately leaves it alone — which made the two steps
  contradict each other, with no version that could satisfy both. Unpublished crates are now exempt
  from the own-version check; their inter-crate pins are still checked
- **Oracle's empty string literal is NULL, and the analyzer did not know it.** Oracle is alone in
  treating `''` as NULL, so `SELECT COALESCE(email, '') AS email_or_empty` was typed non-optional —
  `String` rather than `Option<String>` — and the driver could not decode the first row where Oracle
  returned NULL. The literal now carries the right nullability under the Oracle dialect, which
  propagates through `COALESCE`, `CASE ... ELSE ''` and concatenation. The other five dialects are
  unaffected. `NVL` is not covered, though it was already optional by a different route. Found by the
  live Oracle conformance leg; a model-only fixture could never have caught it, since it compares the
  analyzer against the same model it is built from
- **`typescript-duckdb` read every row positionally while indexing it by name.** The generated code
  called `getRows()`, which returns positional arrays, then read fields by property name — so every
  field came back `undefined` at runtime, with no `tsc` error because the row cast is unchecked. It
  now calls `getRowObjects()`. Present since the backend shipped in 0.6.0; there is no
  `typescript-duckdb` integration project, which is why nothing caught it
- **`typescript-oracledb` cast nullable columns to their non-null type.** Three sites — both read
  paths and the `RETURNING` out-binds path — cast to the column's base type while the declared
  interface said `| null`, so a null column was typed as though it could never be null
- **The TypeScript discriminated-union row type omitted some columns entirely.** With
  `outer_join_unions = true`, a column belonging to a join group that carries no discriminant — one
  where every projected column was already nullable in the schema — matched neither the base-field
  loop nor any union variant, so it was declared nowhere. A query selecting five columns produced a
  row type declaring three. The Zod form of the same function had the identical defect, despite its
  contract that the two shapes cannot drift apart
- **Provenance verification could discard every lint finding.** A target whose backend and engine
  pair failed to construct — a config with no `[[sql.gen]]` block synthesizes a `rust-sqlx` target,
  which does not support every engine `check` accepts — aborted the run before findings were emitted,
  so `check` exited with an error and an empty SARIF report while real findings existed. Verification
  now cannot unwind past emission. Related: the header was compared against the raw backend alias
  from the config rather than the canonical name, so a target written as `sqlx` reported backend
  drift against its own output forever
- **`task version:sync` invalidated every generated artifact.** The provenance header embeds the
  scythe version, so bumping it made all committed artifacts stale and failed the generated-freshness
  gate on the release commit itself — on every future release, not just this one. `version:sync` now
  regenerates after bumping and rewrites the version in documented examples, and CI carries a guard
  comparing every committed header against the workspace version

## [0.13.0] - 2026-08-07

This release makes generation depend on committed inputs rather than on where the command was run
from, and widens distribution to Node and Python.

### Added

- npm and PyPI wrapper packages for the CLI, so Node and Python teams can pin scythe as a dev
  dependency without installing a Rust toolchain. The npm package is
  [`scythe-cli`](https://www.npmjs.com/package/scythe-cli) and the PyPI package is
  [`scythe-sql`](https://pypi.org/project/scythe-sql/); both expose the binary as `scythe`. Each
  resolves the host platform to a release target triple, downloads the matching asset, verifies its
  SHA-256 against the release checksums file, and unpacks it. They are shims over assets the release
  already produced, so the build matrix is unchanged ([#80](https://github.com/Goldziher/scythe/issues/80))
- The two platform gaps are handled deliberately rather than silently. musl Linux has no published
  asset and the gnu binary dies at exec with an opaque loader error, so it fails at install time
  naming the platform. Windows on ARM also has no asset, but the x64 binary runs under emulation, so
  it falls back with a warning rather than blocking a configuration that works
- Both wrappers work behind a corporate proxy and a TLS-intercepting one: they honour `HTTPS_PROXY`,
  `NO_PROXY` and npm's or pip's own proxy settings, and read a CA bundle from `npm_config_cafile` or
  `NODE_EXTRA_CA_CERTS`. The cached binary is written through a temp file and renamed into place, so
  an interrupted install leaves no truncated binary for the next run to trust
- `task version:check` fails on either a crate whose own version disagrees with `scythe-cli`'s or an
  inter-crate pin that does. `version:sync` now runs it, so a partial version bump can no longer
  reach a tag

### Fixed

- **`scythe.toml` paths resolved against the working directory, so `--config` was close to unusable
  from outside the project.** `scythe generate --config /path/to/project/scythe.toml` run from
  anywhere else silently found no schema and no queries, and a `scythe.toml` could not describe its
  own project independently of where it was invoked from. Paths now resolve against the directory
  containing the config file — see Breaking Changes below ([#84](https://github.com/Goldziher/scythe/issues/84))
- **`task generate:all` regenerated with whatever `scythe` was first on `$PATH`.** With an older
  release installed via `cargo install`, it silently rewrote committed output backwards, printed
  `Done.` per backend and exited 0. During the 0.12.0 release this reverted four files from the
  SQLite `int64` fix back to `int32`, and the damage was indistinguishable from legitimate
  regeneration. The task now builds `scythe-cli` from the workspace and invokes it by absolute path,
  matching what CI's freshness job already did ([#85](https://github.com/Goldziher/scythe/issues/85))
- **`task version:sync` silently skipped `scythe-inspect`**, which was absent from both `sed` file
  lists and missing from the second `sed`'s crate alternation. Because `publish_crates` tolerates an
  "already exists" error from crates.io, a release would publish five crates, skip the sixth and
  report success — while the published `scythe-cli` declared a dependency on the *previous*
  `scythe-inspect`, which resolves fine and therefore never surfaces
- Snowflake typed the same underlying storage three different widths depending on spelling. Snowflake
  aliases every integer spelling — `INT`, `INTEGER`, `BIGINT`, `SMALLINT`, `TINYINT` — to
  `NUMBER(38,0)`, but only `BIGINT` resolved to `int64`; `SMALLINT` and `TINYINT` fell through to
  `int16` and `INT`/`INTEGER` to `int32`. `REAL` and `FLOAT4` likewise reported `float32` for what
  Snowflake stores as an 8-byte double. See Breaking Changes
  ([#83](https://github.com/Goldziher/scythe/issues/83))
- `NUMBER(p,s)` with a non-zero scale is now scale-aware. Through the DDL path this was already
  handled upstream by `normalize_data_type`, but the AST path — `CAST` and parameter inference —
  reached the bare `number` arm and typed a decimal as `int64`. `NUMBER(p,0)` and bare `NUMBER` keep
  `int64`
- Types that had no arm at all and fell through as unknown, erroring only later at the backend
  boundary: `INT1`, `HUGEINT`/`UHUGEINT` (DuckDB), `MONEY`/`SMALLMONEY` (MSSQL and PostgreSQL),
  `XML` (MSSQL and PostgreSQL), `BYTEINT` (Snowflake's synonym for `TINYINT`) and
  `GEOGRAPHY`/`GEOMETRY`. The 128-bit DuckDB integers map to `decimal` rather than a silently
  truncating `int64` — no integer neutral type is wide enough. `GEOGRAPHY`/`GEOMETRY` had been
  documented as `string` since the Snowflake page was written, with no code behind it
- **An explicit `NUMBER(p,0)` resolved to `decimal` through the schema path**, because catalog
  normalization rewrote every two-token `NUMBER(p,s)` to `numeric(p,s)` regardless of the scale, so
  a zero scale reached the decimal arm. `NUMBER(38,0)` is exactly what Snowflake's `DESCRIBE TABLE`
  reports for `INT`, so a schema reverse-engineered from a live table typed its keys `decimal` while
  the same table written with `INT` typed them `int64` — the spelling-dependent inconsistency #83
  set out to end, still reachable by a different route. `oracle.md` had documented the correct
  behaviour (`NUMBER(*, 0)` → `int64`) all along ([#86](https://github.com/Goldziher/scythe/issues/86))
- The unit test guarding zero-scale `NUMBER` passed a hand-built string straight to the conversion
  function, which is not the spelling the schema path produces — so it stayed green while real
  schemas resolved the other way. The type-mapping regressions now drive the whole pipeline
- `verify-binstall` never ran on a release re-run, because `release_assets` is *skipped* rather than
  successful when the assets already exist
- The release workflow's "already published" probe against crates.io sent no `User-Agent`, which
  their data-access policy answers with `403`. The check therefore always reported "not published",
  so a re-run replayed the full publish chain and its 30-second inter-crate sleeps
- The documented pre-commit `rev:` pins in the guide were bumped by nothing and validated by nothing,
  so they still pointed at the previous release. `version:sync` now rewrites them and `version:check`
  fails on drift

### Changed

- `UNSIGNED` is now documented on the MariaDB page, which inherits it from the MySQL dialect but
  never mentioned it

### Breaking Changes

- **Paths in `scythe.toml` resolve against the config file's directory, not the process working
  directory.** This covers `schema` and `queries` glob patterns and `[[sql.gen]].output`. Absolute
  paths and patterns are unchanged, and a config invoked from its own directory — the overwhelmingly
  common case, including every project in this repository — behaves identically. Output moves with
  the inputs: leaving it working-directory-relative would turn a silent wrong read into a silent
  wrong write, scattering generated trees into whatever directory the command ran from. A pattern
  matching nothing is now a hard error naming the pattern, the config directory and the resolved
  pattern, since after this change an empty match is the most common symptom of a stale path
  ([#84](https://github.com/Goldziher/scythe/issues/84))
- **Snowflake `SMALLINT`, `TINYINT`, `INT` and `INTEGER` now generate 64-bit integers**, and `REAL`
  and `FLOAT4` now generate 64-bit floats. Regenerate; in the statically typed targets this changes
  field types and driver accessors (`int` → `long`, `setInt` → `setLong`). Languages that map both
  widths to one native type — Python, TypeScript, PHP — are unaffected
  ([#83](https://github.com/Goldziher/scythe/issues/83))

## [0.12.0] - 2026-08-07

### Added

- `typescript-kysely` backend targeting [Kysely](https://kysely.dev)'s `sql` template tag instead of a specific driver, so one generated file runs against any Kysely dialect. Covered end to end against all four built-in dialects — `PostgresDialect`, `MysqlDialect` (mysql2), `SqliteDialect` (better-sqlite3) and `MssqlDialect` (tedious + tarn) — plus a MariaDB project, each running against a live container. A `redshift` manifest also ships, but has no integration project of its own — Redshift coverage is inherited from the wire-compatible PostgreSQL path, not directly tested. Supports the same `row_type` (interface/zod) and `outer_join_unions` options as the other TypeScript backends; the latter makes scythe's Kysely output strictly more precise than a hand-written Kysely query can express, since Kysely has no way to know a joined column's nullability is correlated with its schema `NOT NULL` constraint ([#66](https://github.com/Goldziher/scythe/issues/66))
- `typescript-node-sqlite` backend targeting Node's built-in `node:sqlite` module (`DatabaseSync`), with zero npm dependencies. Requires Node 23.4+ to run unflagged (`--experimental-sqlite` on Node 22). `DatabaseSync` has no `transaction()` helper, so `:batch` queries emit explicit `BEGIN`/`COMMIT` with a rethrowing `ROLLBACK` ([#66](https://github.com/Goldziher/scythe/issues/66))
- `typescript-wasm-sqlite` backend targeting the official [`@sqlite.org/sqlite-wasm`](https://www.npmjs.com/package/@sqlite.org/sqlite-wasm) build via its synchronous OO1 API ([#66](https://github.com/Goldziher/scythe/issues/66))
- Both new SQLite backends generate **fully synchronous** code — no `async`, `await` or `Promise` anywhere, asserted in tests. This was the explicit ask in [#66](https://github.com/Goldziher/scythe/issues/66): these clients are synchronous, and routing them through Kysely "introduces async promise thrashing"
- `structs_only` now applies to every TypeScript backend, not just `rust-sqlx`. It suppresses the query functions and the driver import while still emitting interfaces, Zod schemas, enums and composites. Combined with `row_type = "zod"` this produces the types-only output requested in [#66](https://github.com/Goldziher/scythe/issues/66)
- `scythe check --database-url` verifies inferred query types against a live PostgreSQL database using the extended query protocol (Parse/Describe), reporting mismatches as rules SC-VER01 through SC-VER05 ([#65](https://github.com/Goldziher/scythe/issues/65))
- Opt-in `outer_join_unions` for the TypeScript backends: outer-join nullability is expressed as a discriminated union rather than independent per-column optionals, so a shape like `{ total: null, notes: "gift" }` — unreachable when `orders.total` is `NOT NULL` — is no longer admitted by the generated type. Now supported for `row_type = "zod"` as well, emitting a real `z.union([...])` ([#64](https://github.com/Goldziher/scythe/issues/64))
- Generated TypeScript is now type-checked with `tsc --noEmit` in every TypeScript integration project and in CI. The strict `tsconfig.json` in those projects was previously decorative: `tsx` only transpiles, and the validation harness ran biome alone. This gate immediately caught several codegen defects listed under Fixed
- SQLite coverage in the tool-validation suite via a new `sqlite_backend_test!` macro. That suite previously exercised only PostgreSQL, MySQL and DuckDB, so `typescript-better-sqlite3` had never been checked against the real TypeScript toolchain
- `cargo binstall` support
- `typescript-snowflake` and `go-gosnowflake` integration coverage against the shared fakesnow server ([#27](https://github.com/Goldziher/scythe/issues/27), [#61](https://github.com/Goldziher/scythe/issues/61))
- End-to-end coverage for `outer_join_unions` (interface and Zod forms) and `structs_only`, which previously had none — every assertion was a unit test over hand-built columns, so no generated project set either option, `tsc` never checked their output and no database ever ran it. `typescript-kysely`'s test now calls all of its generated functions; six were imported and never invoked
- A CI job that regenerates all integration code and fails if the tree moved. The integration jobs test the committed files and never regenerate, so drift meant CI was validating output no current build produces — which is exactly how the `ruby-trilogy` and `go-godror` defects above stayed hidden
- Criterion benchmarks over the analyzer and codegen path (`task bench`). The repo previously had none, so allocation changes in the hot path were unmeasurable
- fakesnow now serves snowflake-jdbc its native Arrow result format, dispatching per session on the login request's `CLIENT_APP_ID` so the Node, Go and .NET drivers keep the JSON path unchanged

### Fixed

- **The integration-test generator's templates had been silently corrupted since July.** The commit that migrated linting to poly ran its prose formatter over `tools/integration-test-generator/templates/*.jinja`, because that config carried no exclusion for `.jinja` files — one was added later, but the damage was never repaired. The formatter stripped every newline and re-wrapped all seven language templates at 120 columns, which split string literals across lines and left `//` comments swallowing the code behind them. Word counts were identical before and after, so nothing was lost; the templates are restored from the last good revision with every subsequent change re-applied. The corruption stayed invisible for a month because it only reaches the tree when someone regenerates, and it surfaced as unrelated-looking compiler errors when they did. `scripts/check-generated-syntax.sh` now parses every generated harness, and the freshness job regenerates the scaffolding as well as the query code so drift in either is caught ([#72](https://github.com/Goldziher/scythe/issues/72))
- **`scythe inspect` was unusable against every live database.** Check SC-INS13 called `round(float8, integer)`, which PostgreSQL does not define, raising SQLSTATE 42883 on every server version. Because `run_all()` was fail-fast, that one bad check aborted the entire inspection. The check is repaired and `run_all()` now isolates a failing check instead of abandoning the run
- `:batch` queries emitted TypeScript that does not compile. When a generated signature exceeded 80 characters the line-wrapping helper discarded the signature its caller passed and rebuilt one from the query's own per-column params — so the function declared `(db, name, email)` while its body referenced `items`, an identifier that was never declared. Affected seven TypeScript backends; any `:batch` query with two or more params and a long enough name was broken
- Oracle `CLOB`, `BLOB`, `NCLOB` and `BFILE` columns failed at runtime in the `rust-sibyl` backend with `Interface("cannot return as a String")`. These are now read through a LOB locator, and the read loop no longer discards the returned byte count (a short read previously truncated large LOBs silently)
- Generated TypeScript did not escape SQL spliced into template literals. A backtick — idiomatic identifier quoting in MySQL/MariaDB, `` `users`.`id` `` — terminated the literal early, a backslash escaped the following character, and a literal `${` opened a live interpolation (inside a Kysely `sql` tag, a parameter binding). Newly reachable once Kysely made MySQL a supported target
- Elixir TDS encoded a `nil` boolean parameter as `0` rather than NULL, because `nil` is falsy in Elixir — silently writing `false` where the caller meant "unknown". Also corrects the MSSQL type map: `float32`/`float64` were mapped to `:decimal` and `date` to `:datetime` ([#28](https://github.com/Goldziher/scythe/issues/28))
- `SUM` and `AVG` result types now follow engine semantics instead of echoing the argument type: `sum(int)` widens to `int64`, `sum(bigint)` to `decimal`, and `avg(int|numeric)` yields `decimal`, matching PostgreSQL
- `scythe check --database-url` prepared every `[[sql]]` block against the PostgreSQL connection regardless of the block's configured engine, so a MySQL or MSSQL block produced a flood of spurious SC-VER01 errors and a non-zero exit. Non-PostgreSQL blocks are now skipped with a warning, as the flag's own documentation already promised
- `scythe check --database-url` printed the connection URL — including the password — into stderr and CI logs on a connection failure. Credentials are now redacted
- Type verification treated `string`, `uuid`, `json` and `inet` as mutually interchangeable, so an inferred `uuid` against a reported `json` passed silently — exactly the wrongly mapped catalog type SC-VER03 exists to catch. Only one-directional widening to `string` is now accepted, and PostgreSQL domain types resolve to their base type
- Boolean codegen options (`outer_join_unions`, `structs_only`, and others) silently coerced any unrecognised value to `false`, so `"on"`, `"TRUE"` or a typo disabled the feature without complaint. They are now parsed strictly and reject invalid values with an error naming the option
- `csharp-oracle` emitted code that does not compile: `OracleDecimal` has no `ToDecimal()`, `bytes` parameters used the wrong `OracleDbType`, and binary columns used a reader method that does not exist for `byte[]`
- The Zod emitter mapped every `bytes` column to `z.instanceof(Buffer)` regardless of backend — wrong for the two new SQLite backends, whose drivers yield `Uint8Array`, and unresolvable in a browser where `Buffer` does not exist. The grouped-struct Zod emitter additionally bypassed the column-aware path entirely, degrading an enum column to a bare `z.string()` instead of referencing its generated enum schema
- `typescript-better-sqlite3`'s integration test was a no-op: the harness template had import and connection branches but no test body, so `main()` was a bare `process.exit` and the CI step passed while exercising nothing. It now runs a full create/read/update/delete round trip with assertions
- The integration-test config templates emitted invalid TOML
- Kysely's `:batch` threw at runtime when handed an existing transaction. `Transaction<DB> extends Kysely<DB>`, so the unconditional `db.transaction()` call nested a transaction inside itself; it now reuses the active one via `db.isTransaction`
- CI's `poly fmt --check` went red repeatedly because `cargo-sort` (run by `poly lint`) rewrites `Cargo.toml` with a 4-space indent while poly's formatter insists on 2, so the two rewrote each other indefinitely. `Cargo.toml` is now excluded from poly's formatter, and `task update` reformats after a dependency bump
- `INSERT INTO t VALUES (...)` with no column list registered no parameters at all — the generated function took none while the SQL carried placeholders, so it compiled and then failed at runtime on a bind-count mismatch. Values now bind positionally to the table's columns in catalog order. Placeholders nested inside function calls or `CASE` branches in `INSERT ... VALUES` and `UPDATE ... SET` were swallowed by a catch-all match arm and are now collected, keeping the target column's type, name and nullability. Contributed by [@Zniece](https://github.com/Zniece) ([#67](https://github.com/Goldziher/scythe/pull/67))
- `ruby-trilogy` quoted only `enum::*` and `string` parameters, so `uuid`, `date`, `time`, `datetime`, `json`, `inet`, `bytes` and `interval` were interpolated bare — a UUID rendered as `WHERE id = f3373249-...`, which MySQL parses as an identifier. It is also the one Ruby backend that builds SQL by interpolation rather than binding, so any value containing an apostrophe broke the statement and was injectable. Trilogy's client has no bind API (its C extension defines only `query`, `query_with_flags` and `escape`), so every non-numeric value now goes through `Trilogy#escape`, with temporal types `strftime`-formatted and `json` serialised first
- `go-godror` declared Oracle `RETURNING ... INTO` OUT variables from the column's neutral type alone, ignoring nullability, then assigned them to pointer fields — code that does not compile. Nullable numeric and temporal params now bind `sql.NullInt32`/`NullInt64`/`NullFloat64`/`NullTime`; string-like columns keep a plain OUT var and take its address, since godror documents `sql.NullString` as unsupported (Oracle cannot distinguish `''` from NULL)
- `java-jdbc` and `kotlin-jdbc` read temporal columns with `rs.getObject(col, LocalDateTime.class)`, which snowflake-jdbc does not implement — its `getObject(int, Class<T>)` dispatches only on the legacy `java.sql` temporal types, so every `TIMESTAMP_NTZ` or `TIMESTAMP_TZ` read threw against real Snowflake. Snowflake now uses the legacy getters with null-safe conversion; the other eight JDBC engines are unchanged
- Oracle schemas written as SQL\*Plus scripts failed to parse, freezing two integration projects at output an old build produced. The statement splitter only recognised `;`, but such scripts terminate on a lone `/` and contain no top-level semicolons; `CREATE SEQUENCE ... START WITH ... INCREMENT BY ...` is also rejected by the parser, and trigger bodies are PL/SQL it cannot read. Sequences and triggers contribute no columns and are now skipped, as `CREATE SCHEMA` already was
- `validate_structural` knew 31 of the 52 registered backends; the other 21 reported "unknown backend" instead of being checked. Enrolling them surfaced two real defects: `python-oracledb`, `python-pyodbc` and `python-snowflake` emitted lines that violate ruff's line length for any query with a few columns, and `typescript-snowflake` emitted `any` for row and bind types
- `scythe check` retained every analyzed query and verifiable block even when `--database-url` was absent, though both are only read when it is set
- **CI's generated-code freshness gate had three holes that let real drift through unnoticed.** `integration_tests/Taskfile.yaml` hand-maintained a list of backends to regenerate that had drifted to 71 of the generator's own `build_backends()` 99 entries, so every Redshift, MSSQL and Snowflake backend was silently excluded and never regenerated by CI; the list now comes from a new `--list` flag on the generator instead of a hand-copied one. The gate also compared with `git diff --exit-code`, which cannot see untracked files, so a newly added backend with no committed output yet would pass green with nothing to regenerate against — it now uses `git status --porcelain`. And `scripts/check-generated-syntax.sh` printed `SKIPPED` and returned success whenever a language checker binary was missing, while the job that ran it installed no PHP, Ruby, Python or Go toolchain at all, so every one of those checks had been silently doing nothing since it was added. The script now fails on a missing checker in `--strict` mode (implied by `CI=true`), and the job installs all four toolchains. Regenerating the 29 backends the gate had never covered turned up no behavioral drift — 13 files changed, all formatting ([#72](https://github.com/Goldziher/scythe/issues/72))
- Bare `FLOAT` (no precision) normalized incorrectly on two dialects, though not the defect [#73](https://github.com/Goldziher/scythe/issues/73) reported: the silent narrowing of `FLOAT(53)` it described does not exist, since `normalize_data_type` already maps any precision above 24 to `double precision` on every dialect. The real bugs were in the two unparameterized paths. The neutral-type resolver's bare `"float"` string arm defaulted to `float32` unconditionally, wrong on PostgreSQL, MSSQL and Oracle, where bare `FLOAT` is 8-byte double precision; and `DataType::Float(None)` — sqlparser's node for the same bare `FLOAT` — normalized to `double precision` for every dialect, wrong on MySQL, whose bare `FLOAT` is a genuine 4-byte type. Both paths are now dialect-aware: MySQL's bare `FLOAT` resolves to `float32`, every other dialect to `float64`
- MySQL `UNSIGNED` integer columns (`INT UNSIGNED`, `BIGINT UNSIGNED`, etc.) failed codegen outright with `BackendError::UnknownType("bigint unsigned")` — a hard failure on an ordinary MySQL schema, not a silently wrong type. `normalize_data_type` preserved the MySQL display width (e.g. `"bigint(20) unsigned"`), which the neutral-type resolver had no matching arm for and which its `strip_precision` helper couldn't rescue, since the parenthesized width isn't trailing. The normalizer now discards the display width for every unsigned integer type, giving the resolver a clean string to match ([#74](https://github.com/Goldziher/scythe/issues/74))
- `BIT` columns lost their declared width during normalization, collapsing `BIT(1)` and `BIT(n>1)` to the same bare `"bit"` string and making them indistinguishable downstream. The width is now preserved (`"bit(1)"`, `"bit(8)"`), so the neutral-type resolver can tell a boolean-ish `BIT(1)` from a genuine multi-bit value — which PostgreSQL treats as a bit string (`bytes`) and MySQL treats as an integer bitfield (`int64`). MSSQL's `BIT` is untouched: it has no width and was already correctly `bool`, and none of the 11 MSSQL integration harnesses changed when regenerated, which is the evidence the fix is scoped to PostgreSQL and MySQL ([#75](https://github.com/Goldziher/scythe/issues/75))
- A scalar subquery's inferred type inherited only its projected column's own nullability, ignoring that the subquery itself evaluates to `NULL` when it matches zero rows: `(SELECT name FROM users WHERE id = $1)` was typed non-nullable whenever `users.name` was `NOT NULL`, even though the subquery is nullable by construction. It's now nullable unless the query is provably guaranteed to return exactly one row — an ungrouped aggregate — in which case the aggregate's own nullability already accounts for the empty-input case. That carve-out stops short of windowed aggregates: `COUNT(*) OVER ()` looks like a single-row aggregate but produces one output row per input row, so it stays subject to the general zero-rows-means-`NULL` rule ([#76](https://github.com/Goldziher/scythe/issues/76))
- `WITH RECURSIVE` queries were typed from the anchor branch alone, discarding the recursive branch's analysis with `let _ = ...` — under-reporting nullability whenever the recursive term introduced a `NULL` the anchor didn't have, such as a `LEFT JOIN` or an explicit `NULL` literal filling a column the anchor fills with a `NOT NULL` value. The full anchor-UNION-recursive query is now re-analyzed and kept, taking the same `SetOperation` path as any other `UNION` — which also means a column-count mismatch between the anchor and recursive branches now surfaces as an error instead of being silently swallowed ([#77](https://github.com/Goldziher/scythe/issues/77))
- SQLite integration harnesses are retyped to match the widened `int64`/`float64` neutral types from the dialect-aware fixes above. **`rust-sqlx-sqlite` had not compiled since `93b76b9`**, the earlier commit in this same release that widened SQLite `REAL` to `float64`: its harness template still declared `let total: f32`, so `order.total == total` could not type-check against the now-`f64` generated field. CI runs this suite on every push, so the job had been red since that commit landed; a concurrent GitHub Actions outage kept the failure from getting the scrutiny a red required job normally gets. `go-database-sql-sqlite` had the same defect from the `INTEGER` → `int64` change, failing with eight type errors because its harness declared `var createdUserID int32` while the generated code returned `int64`; its template grouped SQLite with MySQL, MSSQL and Snowflake, which legitimately use 32-bit ids, and SQLite now has its own arm. The remaining statically typed SQLite harnesses — C#, Java, Kotlin and the four TypeScript projects — were compile-checked and were already correct. Both fixed here
- Every generated `Gemfile` declared no gem source, so all six Ruby integration projects failed to install. The same formatter pass had joined the magic comment and the source directive onto one line — `# frozen_string_literal: true source "https://rubygems.org"` — and since `#` opens a Ruby comment, the `source` call was silently commented out. The file stayed valid Ruby, which is why it survived: Bundler simply reported that gems were missing "in locally installed gems". CI additionally ran an older Bundler than the committed `Gemfile.lock` files were written with, which an older Bundler cannot read, so the Ruby steps now pin `bundler: latest`
- Every generated `mix.exs` was a syntax error, failing six integration jobs before any Elixir test ran. Its template had been reflowed as prose and re-wrapped at 120 columns, putting the whole module on one line, which Elixir rejected at `mix.exs:1:63`. Formatting only — project name, version, `elixirc_paths` and every dependency are unchanged. Together with `pyproject.toml` below, this is the last functionally broken residue of that formatter pass; `composer.json` and `tsconfig.json` were reflowed by it too but remain valid, since JSON ignores whitespace
- Five Kotlin projects had their generated file committed as `Queries.kt` while the generator writes `queries.kt`. On a case-insensitive filesystem those are one path, so generation overwrote in place and git never recorded the rename — meaning those projects shipped generated code that no current build produces, visible only on Linux. All nine Kotlin projects now use the generator's name. Java keeps `Queries.java`, which is correct, as Java requires the filename to match the public class
- `go-godror-oracle` did not compile: its harness passed `int64` where the generated `CreateOrder` expects `float64`. Oracle's `NUMBER(10, 2)` is a scaled decimal and correctly maps to `float64`, so the harness was stale, not the type mapping
- `go-gosnowflake` failed with missing `go.sum` entries across the driver's Azure, AWS and Arrow dependency tree. It was the only one of the eight Go integration steps not running `go mod tidy`, and since the generated `go.mod` carries direct dependencies only while Go 1.17+ module-graph pruning needs the indirect set resolved, that step could never have succeeded
- Every generated Python integration project had an unparseable `pyproject.toml`. Its template's first line had been collapsed into `[project] name = "..." version = "..." requires-python = "..." dependencies = [`, which looks plausible but is invalid TOML, so `uv` failed with `TOML parse error at line 1, column 11` before running anything. This broke seven of the eight integration jobs — every one that touches a Python backend — and is unrepaired residue from the same formatter pass that reflowed the Jinja templates as prose in `d895d04`; the follow-up repair in `d4d6694` missed this file. The other two TOML templates were checked and are intact
- **Manifest selection no longer depends on the process working directory.** At 56 call sites, one per backend constructor, manifest loading preferred a CWD-relative `backends/<name>/manifest.toml` over the compiled-in manifest, so identical inputs produced different generated code depending on where `scythe` was invoked, with nothing in the output recording which manifest was used. The lookup was also engine-blind: every `rust-sqlx` engine variant probed the same PostgreSQL file, and `java-jdbc` collapsed nine engines onto one path, so a MySQL or SQLite target run from a directory containing `backends/` would have silently received PostgreSQL type mappings. Manifests are now compiled in and selected purely from `(backend, engine)`. The lookup was undocumented — no CLI flag, no config key, no log line, discoverable only by reading the source — and fired in no integration project and no CI job; all 13 stub manifests were byte-identical to their compiled-in counterparts, so removing it changes no generated output. Note that this is the determinism half of [#82](https://github.com/Goldziher/scythe/issues/82) only: there is no user-facing manifest override yet, and the issue stays open for one, since a global override directory would reintroduce the engine collision above. Also corrects the MSSQL type-mapping docs, which claimed `TINYINT` maps to a nonexistent `int8` neutral type — it maps to `int16`

### Changed

- **Breaking (output):** `scythe check` now writes its report to stdout instead of stderr, and no longer prints the trailing `Check passed.` line or the warnings-only summary. Scripts parsing either will need updating
- **Breaking (types):** SQLite `REAL` and `INTEGER` columns now resolve to `float64` and `int64` instead of `float32` and `int32`. Both of SQLite's storage classes are 8 bytes wide with no narrower variant — `REAL` is defined as an 8-byte IEEE float, and `INTEGER` holds up to 8 bytes — but the type mapper applied PostgreSQL's 4-byte widths (`real`/`integer` genuinely are `float4`/`int4` there) to every engine. Statically typed SQLite targets change accordingly (`f32` → `f64`, `int32` → `int64`, `float`/`int`/`getFloat`/`getInt` → `double`/`long`/`getDouble`/`getLong`); because `INTEGER PRIMARY KEY` is SQLite's rowid alias, every SQLite primary key retypes along with it. PostgreSQL is unaffected ([#70](https://github.com/Goldziher/scythe/issues/70))
- **Breaking (API):** generated Kysely functions now take `QueryExecutorProvider` rather than `Kysely<DB>`. The previous `<DB = any>` generic did nothing useful, and the narrower type rejected callers holding a connection or a controlled transaction — both of which Kysely's `RawBuilder.execute` accepts
- The Snowflake `NUMBER(p, s)` normalizer now preserves precision and scale in `Catalog::Column::sql_type` (consumed by the `rust-sibyl` backend). This does **not** change inferred neutral types: `sql_type_to_neutral` strips precision before mapping, so `numeric(10,2)` and `numeric` resolve identically. The `int64` → `float64` shift visible in the Snowflake integration output came from regenerating stale committed files, not from this change. *(Supersedes an earlier entry in this section which credited this commit with fixing money-column truncation across all seven Snowflake projects — that truncation was a real bug, but it was fixed for Oracle in 0.11.0 and the Snowflake output was merely out of date.)*
- `better-sqlite3` moves to `^12.11.1` in the integration projects, and the SQLite CI job to Node 24. `node:sqlite` is only unflagged from Node 23.4, and better-sqlite3 11.10 will not load on Node 24+
- The documentation site migrated from zensical to Astro Starlight, which incidentally fixed `guide/audit` and `guide/inspect` being unreachable on the live site
- fakesnow's shared query-request wrapper (moved to `integration_tests/fakesnow/fakesnow_server.py`, see Removed) now always emits the plain Snowflake JSON rowset format — every cell stringified, matching real Snowflake's wire format — instead of doing so only for the Node driver. Query execution is serialized with an `asyncio.Lock` so a client's HTTP retry cannot re-run a statement concurrently with the still-in-flight original. The login handler now advertises `CLIENT_RESULT_COLUMN_CASE_INSENSITIVE`, which snowflake-jdbc needs to resolve lowercase generated-code column lookups (`getInt("id")`) against fakesnow's uppercase column labels
- `go-gosnowflake`'s generated harness pointed at `sql/snowflake/schema_emu.sql`, a leftover from an abandoned Docker-emulator plan that lacked `AUTOINCREMENT`; it now uses `sql/snowflake/schema.sql` like every other Snowflake backend
- `elixir-tds-mssql` is back in CI — its step was added in `7b3ec72`, removed in `5edaaa6` while the backend was broken, and restored once the type mapping was fixed. Its `integration_tests/Taskfile.yaml` entry is new ([#28](https://github.com/Goldziher/scythe/issues/28))

### Removed

- `integration_tests/typescript-snowflake/fakesnow_server.py` moved to `integration_tests/fakesnow/fakesnow_server.py` — it is shared infrastructure for every non-Python Snowflake driver, not a TypeScript-specific fixture
- The root-level `backends/` directory (23 files): 13 manifests byte-identical to their compiled-in counterparts under `crates/scythe-codegen/manifests/`, plus 10 Jinja templates that no code path reads — a vestige of an abandoned template-based architecture, since every backend emits strings directly. It existed only to be picked up by the working-directory-relative manifest lookup removed above. `scythe-backend`'s renderer tests, the sole remaining reader, now use a fixture under `crates/scythe-backend/tests/fixtures/`
- `naming.field_case` manifest option. It was deserialized from all 106 manifests and never read — field names come from `to_snake_case` in `resolve.rs` regardless of what's declared, so the 73 manifests declaring `camelCase` or `PascalCase` were silently ignored, while `fn_case` on those same manifests is honored, which is what made the gap easy to miss. Implementing it instead would rename fields in generated code for most backends and break every downstream caller that destructures a row, so the option is deleted rather than wired up — universal snake_case field naming is now intentional instead of accidental. Regenerating all 99 integration backends afterward produced no diff, which is the proof the option was dead ([#69](https://github.com/Goldziher/scythe/issues/69))

### Unverified / Skipped in CI

These backends have codegen support but are not exercised against a live database. The equivalent
list under [0.6.8] describes that release and is now out of date; this one supersedes it.

**Snowflake** ([#27](https://github.com/Goldziher/scythe/issues/27)) — `python-snowflake`,
`typescript-snowflake`, `go-gosnowflake`, `java-jdbc-snowflake` and `kotlin-jdbc-snowflake` all run
against the shared [fakesnow](https://github.com/tekumara/fakesnow) server. The two JDBC suites were
unblocked this release by teaching fakesnow to serve snowflake-jdbc its native Arrow result format.
Still excluded:

- `csharp-snowflake` — the codegen defect was fixed this release, but the harness fails earlier:
  Snowflake.Data names its bind parameters `p1`/`p2`/`p3`, which fakesnow's binding-name heuristic
  treats as named rather than positional. A fakesnow limitation, not a codegen one
- `php-pdo-snowflake` — uncoverable. Snowflake has no PDO driver; access requires the proprietary
  closed-source ODBC driver preinstalled on the runner

**Oracle** — `elixir-jamdb` (`DBConnection.ConnectionPool` dispatch error with `jamdb_oracle`) and
`ruby-oci8` (native gem needs Oracle Instant Client SDK headers unavailable in CI).

**SQLite** — `php-pdo-sqlite` has no CI job. The `createUser` arity mismatch noted under [0.6.8] is
resolved (generated signature and harness call now agree, and the harness parses), but it has never
been run against a database.

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

`python-snowflake`, `typescript-snowflake`, and `go-gosnowflake` all run in CI against a shared
[fakesnow](https://github.com/tekumara/fakesnow) server
(`integration_tests/fakesnow/fakesnow_server.py`) — gosnowflake connects with `protocol=http&insecureMode=true`
to skip TLS/OCSP the same way the Node driver does. The remaining three are still excluded:

- `java-jdbc-snowflake` / `kotlin-jdbc-snowflake` — both use the snowflake-jdbc driver, which fakesnow forces
  into its JSON result format (fakesnow has no Arrow chunk-download endpoint). snowflake-jdbc's JSON-format
  `ResultSet` doesn't implement `getObject(int, LocalDateTime.class)`, so any query touching a `TIMESTAMP_NTZ`
  column throws regardless of how the connection is configured. Enabling these needs an Arrow chunk-download
  endpoint in `fakesnow_server.py`; the JDBC-side wiring (insecure TLS URL parameters) was deliberately not
  landed, since it can't be verified end to end until that blocker is lifted.
- `csharp-snowflake` — not attempted. The Snowflake.Data driver needs its own TLS/OCSP and result-format
  investigation, which was not carried out, so no claim is made either way about whether it can work.
- `php-pdo-snowflake` — genuinely uncoverable in CI: `composer.json` only declares `ext-pdo_odbc`, and the
  proprietary `pdo_snowflake` PHP extension isn't installable through Composer, PECL, or any standard CI
  package manager. It requires Snowflake's closed-source ODBC driver preinstalled on the runner.

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
