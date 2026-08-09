---
title: Formatting
description: SQL formatting via scythe's sqruff integration.
---

Scythe integrates [sqruff](https://github.com/quarylabs/sqruff) for SQL formatting.

## Basic Usage

```bash
# Format all SQL files in your project (reads from scythe.toml)
scythe fmt

# Format specific files
scythe fmt sql/queries.sql sql/schema.sql

# Check formatting without modifying files (exit 1 if changes needed)
scythe fmt --check

# Show a diff of what would change
scythe fmt --diff
```

## Dialect Selection

```bash
# Use a specific SQL dialect for formatting rules
scythe fmt --dialect postgres
scythe fmt --dialect mysql
scythe fmt --dialect ansi
```

If `--dialect` is not given, scythe falls back to the first `[[sql]].engine` in `scythe.toml` (mapped
to its sqruff dialect); only when no config resolves a dialect either does it fall back to `ansi`. When
using a config file, both query files and schema files are included.

`scythe fmt` always runs sqruff's default rule set and ignores `[lint.sqruff]` entirely -- a rule
turned `"off"` there for `scythe lint` still runs (and can still rewrite files) under `scythe fmt`.
The one exception is `LT01`, which is excluded under both `scythe lint` and `scythe fmt` -- it splits
compound operators like `>=` and `<@` into separate tokens (an upstream sqruff bug), so scythe
excludes it unconditionally rather than let it corrupt every formatted file.

## CI Integration

Use `--check` in CI pipelines to enforce formatting:

```bash
scythe fmt --check
```

This exits with code 1 if any files need formatting, making it suitable for CI checks.

## Example

Before formatting:

```sql
select u.id,u.name,o.total from users u left join orders o on u.id=o.user_id where u.status=$1
```

After `scythe fmt`:

```sql
SELECT
    u.id,
    u.name,
    o.total
FROM users u
LEFT JOIN orders o
    ON u.id = o.user_id
WHERE u.status = $1
```

## Formatting + Linting

`scythe fmt` handles whitespace and formatting. `scythe lint` handles logical and structural rules. Run both:

```bash
scythe fmt
scythe lint
```

Or combine formatting with lint auto-fix:

```bash
scythe fmt
scythe lint --fix
```

## Pre-commit Hook

Scythe provides a pre-commit hook for automatic formatting on commit. See [Pre-commit Hooks](/scythe/guide/pre-commit-hooks/) for setup instructions.
