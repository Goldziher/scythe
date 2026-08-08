---
title: Migration from sqlfluff
description: Switch from sqlfluff to scythe -- command mapping, config format, and rule codes.
---

If you use sqlfluff for SQL linting, scythe integrates [sqruff](https://github.com/quarylabs/sqruff) — a Rust reimplementation of sqlfluff — and adds 23 codegen-aware rules (plus 35 further `scythe audit` security/migration rules that also run under `scythe lint`).

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

[lint.sqruff.rules]
"LT01" = "off"
"LT02" = "off"
```

There is no `exclude_rules` key in scythe's `[lint.sqruff]` -- set each rule's status individually
under `[lint.sqruff.rules]` instead, using the bare sqruff code (not the `SQ-` prefix scythe uses in
its own output). See [Linting](/scythe/guide/linting/#sqruff-configuration) for the full field
reference.

## Rule codes

sqruff uses the same rule codes as sqlfluff (LT01, CP01, AM01, etc.). Not all sqlfluff rules are implemented in sqruff. Check the [sqruff repository](https://github.com/quarylabs/sqruff) for the current supported set.

## What scythe adds

scythe's 23 rules (`SC-S*` safety, `SC-C*` codegen, `SC-N*` naming, `SC-A*` antipattern, `SC-P*` performance, `SC-T*` style) use schema and type information that sqlfluff does not have access to. These catch issues like:

- UPDATE/DELETE without WHERE (SC-S01/S02)
- Ambiguous columns in JOINs (SC-S06)
- Comparing with NULL instead of IS NULL (SC-A01)
- ORDER BY without LIMIT (SC-P01)

See [Lint Rules Reference](/scythe/reference/lint-rules/) for the full list.
