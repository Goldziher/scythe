# Lint Rules Reference

Scythe has 22 built-in rules plus 71 sqruff rules (93 total).

## Scythe Rules

### Safety (SC-S)

| Rule | Default | Description |
|------|---------|-------------|
| `SC-S01` | error | UPDATE without WHERE affects all rows |
| `SC-S02` | error | DELETE without WHERE affects all rows |
| `SC-S03` | warn | SELECT * makes queries fragile when columns change |
| `SC-S04` | warn | Declared parameter placeholders ($N) not all used |
| `SC-S05` | warn | Non-deterministic ORDER BY with LIMIT |
| `SC-S06` | warn | BETWEEN with timestamps may miss edge cases |

### Naming (SC-N)

| Rule | Default | Description |
|------|---------|-------------|
| `SC-N01` | warn | Table names should be snake_case |
| `SC-N02` | warn | Column names should be snake_case |
| `SC-N03` | warn | Query names should start with a verb (Get, List, Create, Update, Delete) |
| `SC-N04` | warn | Avoid reserved SQL keywords as identifiers |

### Style (SC-T)

| Rule | Default | Description |
|------|---------|-------------|
| `SC-T01` | warn | Prefer explicit JOIN syntax over implicit (comma) joins |
| `SC-T02` | warn | Prefer COALESCE over CASE WHEN ... IS NULL |
| `SC-T03` | warn | Prefer COUNT(*) over COUNT(column) unless NULLs matter |

### Performance (SC-P)

| Rule | Default | Description |
|------|---------|-------------|
| `SC-P01` | warn | LIMIT without ORDER BY returns arbitrary rows |
| `SC-P02` | warn | Leading wildcard in LIKE prevents index usage |
| `SC-P03` | warn | NOT IN with subquery -- use NOT EXISTS instead |

### Antipattern (SC-A)

| Rule | Default | Description |
|------|---------|-------------|
| `SC-A01` | error | Using = or <> with NULL (use IS NULL / IS NOT NULL) |
| `SC-A02` | warn | Implicit type coercion in comparisons |
| `SC-A03` | warn | OR conditions in JOIN ON clause (usually a mistake) |

### Codegen (SC-C)

| Rule | Default | Description |
|------|---------|-------------|
| `SC-C01` | error | Missing @returns annotation |
| `SC-C02` | error | Duplicate @name in same query file |
| `SC-C03` | warn | @returns :one on query that may return multiple rows |

## sqruff Rule Categories

| Prefix | Category | Rules |
|--------|----------|-------|
| AL | Aliasing | 9 rules |
| AM | Ambiguity | 6 rules |
| CP | Capitalisation | 5 rules |
| CV | Convention | 11 rules |
| JJ | Jinja | 1 rule |
| LT | Layout | 13 rules |
| RF | References | 6 rules |
| ST | Structure | 9 rules |

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

Severity values: `"error"` (blocks generation), `"warn"` (reports only), `"off"` (disabled).
