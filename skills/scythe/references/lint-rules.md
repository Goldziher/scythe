# Lint Rules Reference

Scythe has 23 built-in lint rules and 35 audit rules (58 built-in), plus sqruff's 69 style rules via integration.

The 7 `SC-PRV*` provenance rules and the 7 `SC-DRF*` schema drift rules are not counted in the 58. They
run only from `scythe check` and never appear in `scythe lint` or `scythe audit --list-rules` output.

## Scythe Rules

### Safety (SC-S)

| Rule | Default | Description |
|------|---------|-------------|
| `SC-S01` | error | UPDATE without WHERE affects all rows |
| `SC-S02` | error | DELETE without WHERE affects all rows |
| `SC-S03` | warn | SELECT * makes queries fragile when columns change |
| `SC-S04` | warn | Declared parameter placeholders ($N) not all used |
| `SC-S05` | warn | DML with :one/:many command should have a RETURNING clause |
| `SC-S06` | warn | SELECT with JOIN has unqualified column references |
| `SC-S07` | error | SQL placeholder $N present in query body but absent from the generated parameter signature |

### Naming (SC-N)

| Rule | Default | Description |
|------|---------|-------------|
| `SC-N01` | warn | Column aliases should use snake_case |
| `SC-N02` | warn | Table names should use snake_case |
| `SC-N03` | warn | Query name should start with an action verb |
| `SC-N04` | warn | Table aliases should be lowercase |

### Style (SC-T)

| Rule | Default | Description |
|------|---------|-------------|
| `SC-T01` | warn | Implicit join (FROM a, b WHERE ...) -- prefer explicit JOIN syntax |
| `SC-T02` | warn | CASE WHEN x IS NULL THEN y ELSE x END can be COALESCE(x, y) |
| `SC-T03` | warn | COUNT(1) is equivalent to COUNT(*) -- prefer COUNT(*) for clarity |

### Performance (SC-P)

| Rule | Default | Description |
|------|---------|-------------|
| `SC-P01` | warn | ORDER BY without LIMIT may cause unnecessary sorting of large result sets |
| `SC-P02` | warn | LIKE pattern starting with % prevents index usage |
| `SC-P03` | warn | NOT IN (SELECT ...) has unexpected NULL behavior -- prefer NOT EXISTS |

### Antipattern (SC-A)

| Rule | Default | Description |
|------|---------|-------------|
| `SC-A01` | error | Comparing with NULL using = or != always yields NULL -- use IS NULL / IS NOT NULL |
| `SC-A02` | off | Implicit type coercion may cause unexpected behavior |
| `SC-A03` | warn | OR in JOIN ON condition usually prevents index usage |

### Codegen (SC-C)

| Rule | Default | Description |
|------|---------|-------------|
| `SC-C01` | off | Query should have a @returns annotation (already enforced by parser) |
| `SC-C02` | warn | :exec command but query has RETURNING clause -- returned rows will be discarded |
| `SC-C03` | error | Multiple queries share the same @name |

## Check-time Rules

These two families are not reachable from `scythe lint`.

### Provenance (SC-PRV)

Compares a generated artifact's provenance header against the current schema, engine, backend, and
scythe version. Runs on every `scythe check`, no flag required.

| Rule | Default | Description |
|------|---------|-------------|
| `SC-PRV01` | error | Generated artifact was produced from a different schema than the one on disk |
| `SC-PRV02` | warn | Generated artifact was produced by a different scythe version than the one running |
| `SC-PRV03` | error | Generated artifact was produced by a different backend than this target configures |
| `SC-PRV04` | error | Generated artifact was produced for a different engine than this target configures |
| `SC-PRV05` | warn | Generated artifact has no provenance header, so it cannot be checked for schema drift |
| `SC-PRV06` | warn | Generated artifact's provenance header is missing one or more required fields |
| `SC-PRV07` | warn | Generation target could not be verified: backend construction or artifact read failed |

### Schema drift (SC-DRF)

Compares the committed DDL against a live database catalog. Runs only from
`scythe check --database-url` (PostgreSQL only).

| Rule | Default | Description |
|------|---------|-------------|
| `SC-DRF01` | error | Table declared in the DDL does not exist in the live database |
| `SC-DRF02` | warn | Table exists in the live database but is not declared in the DDL |
| `SC-DRF03` | error | Column declared in the DDL does not exist on the live table |
| `SC-DRF04` | error | Column exists on the live table but is not declared in the DDL |
| `SC-DRF05` | error | Column's DDL type does not match the type the live database reports |
| `SC-DRF06` | error | Column's DDL nullability does not match the live database |
| `SC-DRF07` | error | Enum type's DDL value set does not match the live database |

`SC-DRF02` is the one warning: every migrated database carries a `schema_migrations` or extension
bookkeeping table the committed DDL never declares.

## sqruff Rule Categories

| Prefix | Category | Rules |
|--------|----------|-------|
| AL | Aliasing | AL01-AL09 |
| AM | Ambiguity | AM01-AM09 |
| CP | Capitalisation | CP01-CP05 |
| CV | Convention | CV01-CV12 |
| JJ | Jinja | JJ01 |
| LT | Layout | LT01-LT15 |
| RF | References | RF01-RF06 |
| ST | Structure | ST01-ST12 |

## Configuration

```toml
# Category-level severity
[lint.categories]
safety = "error"
naming = "warn"
performance = "warn"
style = "off"
antipattern = "warn"
codegen = "error"

# Per-rule overrides (takes precedence)
[lint.rules]
"SC-S03" = "off"        # allow SELECT *
"SC-N03" = "error"      # enforce verb naming
"SC-P02" = "off"        # allow leading wildcard LIKE
```

`SC-PRV*` and `SC-DRF*` use the same table. `scythe check` applies `[lint]` to their registries before
resolving severities, so per-rule and per-category overrides work identically:

```toml
[lint.rules]
"SC-PRV02" = "error"    # fail CI on scythe version drift
"SC-DRF02" = "error"    # fail on tables the DDL never declares

[lint.categories]
provenance = "off"      # skip provenance verification
drift = "off"           # skip schema drift checking
```

Severity values: `"error"` (blocks generation), `"warn"` (reports only), `"off"` (disabled).
