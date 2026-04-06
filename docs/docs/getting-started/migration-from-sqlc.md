# Migration from sqlc

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
```

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
