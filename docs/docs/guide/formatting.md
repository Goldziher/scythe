# Formatting

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

If no dialect is specified, `ansi` is used by default. When using a config file, both query files and schema files are included.

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
