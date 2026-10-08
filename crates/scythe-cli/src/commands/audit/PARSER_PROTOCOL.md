# Pinned PostgreSQL parser helpers

`scythe audit --cluster-scope` classifies statements with a real PostgreSQL
grammar, not sqlparser. Because the two pinned libpg_query releases expose
identical C symbols, they cannot be linked into one process: they ship as two
separate executables built at release time by
`build_pg_parser_helpers.sh OUTPUT_DIRECTORY`:

| Helper | libpg_query release | Bundled PostgreSQL parser | Patch |
| --- | --- | --- | --- |
| `scythe-pg15-parser` | `15-4.2.4` | 15.1 | none |
| `scythe-pg18-parser` | `18.1.0` | 18.6 | `patches/libpg_query-18.1.0-plpgsql-json.patch` |

The source archives are pinned by release and SHA256 in the build script, and
the PG18 helper is built from the patched source: PostgreSQL 18's PL/pgSQL JSON
serializer corrupts trigger-function output, drops `RETURN <variable>`, and
refuses schema-qualified types (pganalyze/libpg_query#337). The build asserts
the patch markers are present after applying it, so a patch that fails to apply
aborts the build instead of producing a helper that then fails the release
controls.

The build directory also contains `pg-parser-manifest.json`: it names both
PostgreSQL grammar versions, pinned libpg_query releases/source hashes, the PG18
patch hash, and the resulting helper-binary hashes.
`test_pg_parser_helpers.sh BINARY_DIRECTORY` verifies that manifest against the
actual binaries; `test_pg_parser_release_gate.sh BINARY_DIRECTORY` asserts the
three PG18 regressions are fixed and that PG15 still passes.

## Protocol version 1

A helper takes a single SQL string on stdin (64 MiB maximum). With no argument
it calls `pg_query_parse`; with `--plpgsql` it calls `pg_query_parse_plpgsql` on
a complete `CREATE FUNCTION`, `CREATE PROCEDURE`, or `DO` statement. `--version`
reports the exact linked parser release. A successful parse writes one JSON
object to stdout:

```json
{"protocol":1,"pg_major":18,"pg_version":"18.6","parser_release":"18.1.0","ast":{"stmts":[{"stmt":{"CreateRoleStmt":{}}}]}}
```

`ast` is the unmodified libpg_query JSON parse tree: SQL mode returns an object
with `stmts`; PL/pgSQL mode returns an array of compiled functions. Syntax,
input, and output failures are reported on stderr with nonzero exit, and the
helper rejects SQL that produces zero statements. The caller **must** parse and
validate the complete JSON response, including `protocol`, `pg_major`, and the
expected AST shape, before classifying it; `cluster_scope/response.rs` performs
that validation. Exit 0 alone is not evidence of a valid AST, so a malformed,
incomplete, or absent AST is a fail-closed finding that can never be suppressed
by audit severity or `--exit-zero`.

The top-level SQL AST stores routine bodies as strings; the gate re-parses each
body with the PL/pgSQL grammar and walks the compiled tree for static SQL
(`PLpgSQL_stmt_execsql`) and dynamic SQL (`PLpgSQL_stmt_dynexecute`).

## Packaging

Release archives ship both helpers next to the `scythe` binary. The CLI resolves
them from `SCYTHE_PG_PARSER_DIR`, then the directory holding the running
executable; a missing helper is an error, never a silent skip.

Binary distribution of the helpers must reproduce the full copyright,
conditions, and disclaimer text from both libpg_query `LICENSE` files. The
bundled PostgreSQL parser source carries the PostgreSQL License. Version 15 also
compiles vendored protobuf-c (BSD-style notice in
`vendor/protobuf-c/protobuf-c.c`) and xxHash (BSD-2 in
`vendor/xxhash/xxhash.h`). Version 18 compiles upb (BSD-3 in
`vendor/upb/LICENSE`), utf8_range (MIT in
`vendor/upb/third_party/utf8_range/LICENSE`), and xxHash (BSD-2 in its header).
Packaging must carry those full notices before distributing the helpers.
