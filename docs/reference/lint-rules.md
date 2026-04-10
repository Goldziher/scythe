# Lint Rules

Scythe includes 22 built-in rules and integrates sqruff for additional SQL style and formatting rules.

## Scythe rules (22)

### Safety

| Code | Name | Description | Default |
|------|------|-------------|---------|
| `SC-S01` | `update-without-where` | UPDATE without WHERE affects all rows | Error |
| `SC-S02` | `delete-without-where` | DELETE without WHERE affects all rows | Error |
| `SC-S03` | `no-select-star` | SELECT * makes queries fragile when columns change | Warn |
| `SC-S04` | `unused-params` | Declared parameter placeholders ($N) not all used | Warn |
| `SC-S05` | `missing-returning` | DML with :one/:opt/:many command should have a RETURNING clause | Warn |
| `SC-S06` | `ambiguous-column-in-join` | SELECT with JOIN has unqualified column references | Warn |

### Codegen

| Code | Name | Description | Default |
|------|------|-------------|---------|
| `SC-C01` | `missing-returns-annotation` | Query should have a @returns annotation (enforced by parser) | Off |
| `SC-C02` | `exec-with-returning` | :exec command but query has RETURNING clause -- returned rows will be discarded | Warn |
| `SC-C03` | `duplicate-query-names` | Multiple queries share the same @name | Error |

### Naming

| Code | Name | Description | Default |
|------|------|-------------|---------|
| `SC-N01` | `prefer-snake-case-columns` | Column aliases should use snake_case | Warn |
| `SC-N02` | `prefer-snake-case-tables` | Table names should use snake_case | Warn |
| `SC-N03` | `query-name-convention` | Query name should start with an action verb | Warn |
| `SC-N04` | `consistent-alias-casing` | Table aliases should be lowercase | Warn |

### Antipattern

| Code | Name | Description | Default |
|------|------|-------------|---------|
| `SC-A01` | `not-equal-null` | Comparing with NULL using = or != always yields NULL; use IS NULL / IS NOT NULL | Error |
| `SC-A02` | `implicit-type-coercion` | Implicit type coercion may cause unexpected behavior | Off |
| `SC-A03` | `or-in-join-condition` | OR in JOIN ON condition usually prevents index usage | Warn |

### Performance

| Code | Name | Description | Default |
|------|------|-------------|---------|
| `SC-P01` | `order-without-limit` | ORDER BY without LIMIT may cause unnecessary sorting of large result sets | Warn |
| `SC-P02` | `like-starts-with-wildcard` | LIKE pattern starting with % prevents index usage | Warn |
| `SC-P03` | `not-in-subquery` | NOT IN (SELECT ...) has unexpected NULL behavior; prefer NOT EXISTS | Warn |

### Style

| Code | Name | Description | Default |
|------|------|-------------|---------|
| `SC-T01` | `prefer-explicit-join` | Implicit join (FROM a, b WHERE ...) -- prefer explicit JOIN syntax | Warn |
| `SC-T02` | `prefer-coalesce-over-case` | CASE WHEN x IS NULL THEN y ELSE x END can be COALESCE(x, y) | Warn |
| `SC-T03` | `prefer-count-star` | COUNT(1) is equivalent to COUNT(*) -- prefer COUNT(*) for clarity | Warn |

## Configuration

Override severity per-rule or per-category in `scythe.toml`:

```toml
[lint.rules]
"SC-S03" = "error"      # Promote no-select-star to error
"SC-A02" = "warn"       # Enable implicit-type-coercion
"SC-T03" = "off"        # Disable prefer-count-star

[lint.categories]
safety = "error"         # All safety rules become errors
style = "off"            # Disable all style rules
```

Severity levels: `error`, `warn`, `off`.

Priority: per-rule override > per-category override > default severity.

## Sqruff rules

Scythe integrates [sqruff](https://github.com/quarylabs/sqruff) for SQL formatting and style linting. Sqruff violations are prefixed with `SQ-` followed by the sqruff rule code.

| Category | Rules | Description |
|----------|-------|-------------|
| `AL` | AL01-AL07 | Aliasing rules |
| `AM` | AM01-AM07 | Ambiguity rules |
| `CP` | CP01-CP05 | Capitalization rules |
| `CV` | CV01-CV11 | Convention rules |
| `JJ` | JJ01 | Join rules |
| `LT` | LT01-LT13 | Layout rules |
| `RF` | RF01-RF06 | Reference rules |
| `ST` | ST01-ST09 | Structure rules |

### Selected sqruff rules

| Code | Name | Description |
|------|------|-------------|
| `SQ-AL01` | Implicit aliasing | Explicit aliasing with `AS` keyword |
| `SQ-AL02` | Implicit alias type | Column aliases should be explicit |
| `SQ-AM01` | Ambiguous DISTINCT | DISTINCT used with both DISTINCT and non-DISTINCT columns |
| `SQ-CP01` | Keyword capitalization | SQL keywords should be consistently capitalized |
| `SQ-CP02` | Identifier capitalization | Identifiers should be consistently cased |
| `SQ-CV02` | COALESCE vs IFNULL/NVL | Prefer COALESCE over vendor-specific functions |
| `SQ-LT01` | Trailing whitespace | No trailing whitespace |
| `SQ-LT02` | Indentation | Consistent indentation |
| `SQ-LT04` | Comma position | Leading or trailing commas consistently |
| `SQ-LT09` | SELECT targets | Each column on its own line |
| `SQ-RF02` | Qualified references | References should be qualified in JOINs |
| `SQ-ST05` | No CTEs | CTEs preferred over subqueries |

Sqruff rules are run via `scythe lint` and `scythe fmt`. Fixable violations are auto-corrected by `scythe fmt`.
