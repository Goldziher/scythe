---
title: Lint Rules
description: All 58 built-in scythe rules plus the check-time provenance and drift rules and sqruff style rules.
---

Scythe includes 23 built-in lint rules and integrates sqruff for additional SQL style and formatting
rules. `default_registry()` -- the registry `scythe lint` and `scythe audit --list-rules` both read
from -- also carries the 35 `scythe audit` rules (`SC-SEC*`, `SC-RLS*`, `SC-MIG*`, `SC-CHK01`), so all
58 rules run under `scythe lint` too. See the [audit guide](/scythe/guide/audit/) for the audit-only
catalog.

Two further families ship in their own registries and are **not** counted in that 58: the 8 `SC-PRV*`
[provenance rules](#provenance-rules-8) and the 7 `SC-DRF*` [schema drift rules](#schema-drift-rules-7).
Both fire only from `scythe check`, never from `scythe lint` or `scythe audit --list-rules`, so adding
them to the 58 would advertise rules those commands can never report. Every rule count elsewhere in
these docs refers to the 58; the two check-time families are always stated separately.

## Scythe rules (23)

### Safety

| Code | Name | Description | Default |
|------|------|-------------|---------|
| `SC-S01` | `update-without-where` | UPDATE without WHERE affects all rows | Error |
| `SC-S02` | `delete-without-where` | DELETE without WHERE affects all rows | Error |
| `SC-S03` | `no-select-star` | SELECT * makes queries fragile when columns change | Warn |
| `SC-S04` | `unused-params` | Declared parameter placeholders ($N) not all used | Warn |
| `SC-S05` | `missing-returning` | DML with :one/:many command should have a RETURNING clause (`:opt` does not trigger this rule) | Warn |
| `SC-S06` | `ambiguous-column-in-join` | SELECT with JOIN has unqualified column references | Warn |
| `SC-S07` | `unbound-sql-param` | SQL placeholder $N present in query body but absent from the generated parameter signature | Error |

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

## Provenance rules (8)

`SC-PRV*` rules compare an already-generated artifact's [provenance header](/scythe/guide/cli-reference/#provenance-verification)
against the current schema, queries, engine, backend, and scythe version. They run only from `scythe check`
(no flag required) and are excluded from `scythe lint` and `scythe audit --list-rules` -- neither
command reads generated files, so neither can ever produce one of these findings. They are not part
of the 23 scythe rules or the 58 built-in total quoted above.

| Code | Name | Description | Default |
|------|------|-------------|---------|
| `SC-PRV01` | `schema-drift` | Generated artifact was produced from a different schema than the one on disk | Error |
| `SC-PRV02` | `scythe-version-drift` | Generated artifact was produced by a different scythe version than the one running | Warn |
| `SC-PRV03` | `backend-drift` | Generated artifact was produced by a different backend than this target configures | Error |
| `SC-PRV04` | `engine-drift` | Generated artifact was produced for a different engine than this target configures | Error |
| `SC-PRV05` | `missing-provenance-header` | Generated artifact has no provenance header, so it cannot be checked for schema drift | Warn |
| `SC-PRV06` | `malformed-provenance-header` | Generated artifact's provenance header is missing one or more required fields | Warn |
| `SC-PRV07` | `unverifiable-provenance` | Generation target could not be verified: backend construction or artifact read failed | Warn |
| `SC-PRV08` | `query-drift` | Generated artifact's embedded query fingerprint differs from the current query set | Error |

`SC-PRV*` rules are ordinary registry rules: `scythe check` applies the same `[lint]` table to the
provenance registry that it applies to the SQL rules, so severities are overridable and the whole
family is disableable.

```toml
[lint.rules]
"SC-PRV02" = "error"    # fail CI on scythe version drift too
"SC-PRV05" = "off"      # ignore artifacts with no provenance header

[lint.categories]
provenance = "off"      # skip provenance verification entirely
```

## Schema drift rules (7)

`SC-DRF*` rules compare the committed DDL against a live database's catalog. They run only from
[`scythe check --database-url`](/scythe/guide/cli-reference/#schema-drift) (PostgreSQL only) and are
likewise excluded from `scythe lint` and `scythe audit --list-rules`, and from the 23/58 counts above.

| Code | Name | Description | Default |
|------|------|-------------|---------|
| `SC-DRF01` | `table-missing-from-database` | Table declared in the DDL does not exist in the live database | Error |
| `SC-DRF02` | `table-missing-from-ddl` | Table exists in the live database but is not declared in the DDL | Warn |
| `SC-DRF03` | `column-missing-from-database` | Column declared in the DDL does not exist on the live table | Error |
| `SC-DRF04` | `column-missing-from-ddl` | Column exists on the live table but is not declared in the DDL | Error |
| `SC-DRF05` | `column-type-mismatch` | Column's DDL type does not match the type the live database reports | Error |
| `SC-DRF06` | `column-nullability-mismatch` | Column's DDL nullability does not match the live database | Error |
| `SC-DRF07` | `enum-values-mismatch` | Enum type's DDL value set does not match the live database | Error |

`SC-DRF02` is the one `Warn`: every real database carries objects the committed DDL never declares
(migration ledgers such as `schema_migrations`, extension bookkeeping), so defaulting it to `Error`
would fail the first run against a production database. Every other rule describes the DDL promising
something the database does not deliver, which breaks generated code, so it errors.

Drift severities come from the same `[lint]` table as every other rule:

```toml
[lint.rules]
"SC-DRF02" = "error"    # fail on any undeclared table
"SC-DRF04" = "warn"     # downgrade an undeclared column to a warning

[lint.categories]
drift = "off"           # skip drift checking entirely
```

## Sqruff rules

Scythe integrates [sqruff](https://github.com/quarylabs/sqruff) for SQL formatting and style linting. Sqruff violations are prefixed with `SQ-` followed by the sqruff rule code.

| Category | Rules | Description |
|----------|-------|-------------|
| `AL` | AL01-AL09 | Aliasing rules |
| `AM` | AM01-AM09 | Ambiguity rules |
| `CP` | CP01-CP05 | Capitalization rules |
| `CV` | CV01-CV12 | Convention rules |
| `JJ` | JJ01 | Jinja rules |
| `LT` | LT01-LT15 | Layout rules |
| `RF` | RF01-RF06 | Reference rules |
| `ST` | ST01-ST12 | Structure rules |

### Selected sqruff rules

| Code | Name | Description |
|------|------|-------------|
| `SQ-AL01` | Implicit aliasing | Explicit aliasing with `AS` keyword |
| `SQ-AL02` | Implicit alias type | Column aliases should be explicit |
| `SQ-AM01` | Ambiguous DISTINCT | DISTINCT used with both DISTINCT and non-DISTINCT columns |
| `SQ-CP01` | Keyword capitalization | SQL keywords should be consistently capitalized |
| `SQ-CP02` | Identifier capitalization | Identifiers should be consistently cased |
| `SQ-CV02` | COALESCE vs IFNULL/NVL | Prefer COALESCE over vendor-specific functions |
| `SQ-LT01` | Layout spacing | **Excluded under both `scythe lint` and `scythe fmt`** (upstream sqruff bug splits compound operators like `>=` and `<@`) -- the one rule exclusion `scythe fmt` does not ignore |
| `SQ-LT02` | Indentation | Consistent indentation |
| `SQ-LT04` | Comma position | Leading or trailing commas consistently |
| `SQ-LT09` | SELECT targets | Each column on its own line |
| `SQ-RF02` | Qualified references | References should be qualified in JOINs |
| `SQ-ST05` | No CTEs | CTEs preferred over subqueries |

Sqruff rules are run via `scythe lint` and `scythe fmt`. Fixable violations are auto-corrected by `scythe fmt`.

Scythe prefixes every sqruff finding with `SQ-` in its own output (`SQ-LT02`), but `[lint.sqruff.rules]`
config keys use the bare sqruff code (`"LT02"`) -- see [Linting](/scythe/guide/linting/#sqruff-configuration).
