# Migration from sqlfluff

If you use sqlfluff for SQL linting, scythe integrates [sqruff](https://github.com/quarylabs/sqruff) — a Rust reimplementation of sqlfluff — and adds 22 codegen-aware rules.

## Command replacement

| sqlfluff | scythe |
|----------|--------|
| `sqlfluff lint file.sql` | `scythe lint file.sql` |
| `sqlfluff fix file.sql` | `scythe lint --fix file.sql` |
| `sqlfluff format file.sql` | `scythe fmt file.sql` |

## Configuration

sqlfluff uses `.sqlfluff` (INI format). scythe uses `scythe.toml`:

**.sqlfluff:**

```ini
[sqlfluff]
dialect = postgresql
exclude_rules = LT01,LT02
```

**scythe.toml:**

```toml
[[sql]]
engine = "postgresql"

[lint.sqruff]
exclude_rules = ["LT01", "LT02"]
```

## Rule codes

sqruff uses the same rule codes as sqlfluff (LT01, CP01, AM01, etc.). Not all sqlfluff rules are implemented in sqruff. Check the [sqruff repository](https://github.com/quarylabs/sqruff) for the current supported set.

## What scythe adds

scythe's 22 rules (SC-S01 through SC-T03) use schema and type information that sqlfluff does not have access to. These catch issues like:

- UPDATE/DELETE without WHERE (SC-S01/S02)
- Ambiguous columns in JOINs (SC-S06)
- Comparing with NULL instead of IS NULL (SC-A01)
- ORDER BY without LIMIT (SC-P01)

See [Lint Rules Reference](../reference/lint-rules.md) for the full list.
