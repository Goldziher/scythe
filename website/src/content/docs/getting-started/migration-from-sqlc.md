---
title: Migration from sqlc
description: Convert sqlc projects to scythe with the automated migration tool.
---

Scythe includes an automated migration tool that converts sqlc projects to scythe format.

## One Command

```bash
scythe migrate sqlc.yaml
```

This reads your sqlc config (v1 or v2 format), converts query annotations, and generates a `scythe.toml`.

## What Changes

### Config Format

**sqlc.yaml (before):**

```yaml
version: "2"
sql:
  - schema: "sql/schema.sql"
    queries: "sql/queries.sql"
    engine: "postgresql"
    gen:
      go:
        out: "db"
        package: "db"
```

**scythe.toml (after):**

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries.sql"]
output = "db"

[sql.gen.go]
target = "pgx"
```

`scythe migrate` picks the driver from the engine in your sqlc config: `pgx` for PostgreSQL,
`database-sql` for MySQL, SQLite, DuckDB and SQL Server, `godror` for Oracle, `gosnowflake` for
Snowflake. Change it if you want a different one -- see
[Configuration](/scythe/guide/configuration/) for the full backend list.

If an engine has no scythe backend for your language, `migrate` fails and names both rather than
writing a config that cannot generate.

### Query Annotations

**sqlc format (before):**

```sql
-- name: GetUser :one
SELECT * FROM users WHERE id = $1;

-- name: CreateUser :exec
INSERT INTO users (name, email)
VALUES (sqlc.arg(name), sqlc.arg(email));
```

**scythe format (after):**

```sql
-- @name GetUser
-- @returns :one
SELECT * FROM users WHERE id = $1;

-- @name CreateUser
-- @returns :exec
INSERT INTO users (name, email)
VALUES ($1, $2);
```

### Key Differences

| Feature | sqlc | scythe |
|---------|------|--------|
| Annotation style | `-- name: Foo :one` | `-- @name Foo` + `-- @returns :one` |
| Named parameters | `sqlc.arg(name)` | `$1`, `$2`, ... |
| Config format | YAML | TOML |
| Nullable overrides | Go struct tags | `-- @nullable col1, col2` |
| Non-null overrides | Not supported | `-- @nonnull col1` |
| JSON column types | Not supported | `-- @json data = MyType` |
| Deprecation markers | Not supported | `-- @deprecated Use V2` |

### v1 Config Support

The migration tool also handles sqlc v1 configs with the `packages` format:

```yaml
version: "1"
packages:
  - name: "db"
    path: "internal/db"
    queries: "./sql/query/"
    schema: "./sql/schema/"
    engine: "postgresql"
```

This is converted to the equivalent scythe.toml with glob patterns for directories.

## After Migration

1. Review the generated `scythe.toml`
2. Verify with `scythe check`
3. Generate code with `scythe generate`
4. Run `scythe lint` to catch issues sqlc might have missed

> **Note:** Custom type mappings and ORM-specific extensions need manual review after migration.

## Special sqlc Functions

### sqlc.narg()

sqlc's named arguments (`sqlc.arg()` and `sqlc.narg()`) convert to positional parameters with a
`@param` annotation documenting the original name:

**sqlc:**

```sql
-- name: SearchUsers :many
SELECT * FROM users WHERE name = sqlc.narg('name');
```

**scythe:**

```sql
-- @name SearchUsers
-- @returns :many
-- @param name
SELECT * FROM users WHERE name = $1;
```

`sqlc.narg()` does not currently convert to a `@nullable` annotation -- nullability of the parameter
is unaffected by migration and still needs a manual `@nullable` if the column itself isn't already
nullable.

### sqlc.embed() and sqlc.slice()

These sqlc functions do not have direct equivalents in scythe. After migration:

- `sqlc.embed()` — replace with explicit column selection
- `sqlc.slice()` — replace with `= ANY($1)` array parameter

## Migration Limitations

- Custom type overrides need manual review
- `sqlc.embed()` and `sqlc.slice()` require manual rewriting
- Plugin-specific code generation options are not preserved
- Query names are preserved but may need adjustment for scythe naming conventions
