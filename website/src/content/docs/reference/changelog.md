---
title: Changelog
description: Scythe release history.
---

Scythe follows [Keep a Changelog](https://keepachangelog.com/) and [Semantic Versioning](https://semver.org/).

For the latest changes, see the [CHANGELOG.md](https://github.com/Goldziher/scythe/blob/main/CHANGELOG.md) in the repository root.

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
