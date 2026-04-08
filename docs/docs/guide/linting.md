# Linting

Scythe includes 22 built-in rules plus sqruff integration for SQL style and formatting.

## Running the Linter

```bash
# Lint using config (scythe rules + sqruff rules)
scythe lint

# Lint specific files (sqruff rules only)
scythe lint file1.sql file2.sql

# Auto-fix violations where possible
scythe lint --fix

# Use a specific dialect
scythe lint --dialect postgres
```

## Scythe Rules

### Safety

| Rule | Default | Description |
|------|---------|-------------|
| `SC-S01` | error | UPDATE without WHERE affects all rows |
| `SC-S02` | error | DELETE without WHERE affects all rows |
| `SC-S03` | warn | SELECT * makes queries fragile when columns change |
| `SC-S04` | warn | Declared parameter placeholders ($N) not all used |
| `SC-S05` | warn | DML with :one/:many command should have a RETURNING clause |
| `SC-S06` | warn | SELECT with JOIN has unqualified column references |

### Naming

| Rule | Default | Description |
|------|---------|-------------|
| `SC-N01` | warn | Column aliases should use snake_case |
| `SC-N02` | warn | Table names should use snake_case |
| `SC-N03` | warn | Query name should start with an action verb |
| `SC-N04` | warn | Table aliases should be lowercase |

### Style

| Rule | Default | Description |
|------|---------|-------------|
| `SC-T01` | warn | Implicit join (FROM a, b WHERE ...) -- prefer explicit JOIN |
| `SC-T02` | warn | CASE WHEN x IS NULL can be COALESCE(x, y) |
| `SC-T03` | warn | COUNT(1) is equivalent to COUNT(*) -- prefer COUNT(*) |

### Performance

| Rule | Default | Description |
|------|---------|-------------|
| `SC-P01` | warn | ORDER BY without LIMIT may sort large result sets |
| `SC-P02` | warn | LIKE pattern starting with % prevents index usage |
| `SC-P03` | warn | NOT IN (SELECT ...) has unexpected NULL behavior -- prefer NOT EXISTS |

### Antipattern

| Rule | Default | Description |
|------|---------|-------------|
| `SC-A01` | error | Comparing with NULL using = or != -- use IS NULL / IS NOT NULL |
| `SC-A02` | off | Implicit type coercion may cause unexpected behavior |
| `SC-A03` | warn | OR in JOIN ON condition usually prevents index usage |

### Codegen

| Rule | Default | Description |
|------|---------|-------------|
| `SC-C01` | off | Query should have a @returns annotation |
| `SC-C02` | warn | :exec with RETURNING -- should use :one or :many |
| `SC-C03` | error | Multiple queries share the same @name |

## sqruff Rules

Scythe integrates sqruff rules from [sqruff](https://github.com/quarylabs/sqruff), a Rust-based SQL linter. These rules are prefixed with `SQ-` and cover formatting, style, and correctness. They run automatically alongside scythe rules when using `scythe lint` or `scythe check`.

## Configuration

Configure lint severity in `scythe.toml`:

```toml
[lint]

# Set severity by category
[lint.categories]
safety = "error"        # All safety rules become errors
naming = "warn"         # All naming rules become warnings
performance = "off"     # Disable all performance rules

# Override individual rules
[lint.rules]
"SC-S03" = "off"        # Disable SELECT * warning
"SC-A01" = "error"      # NULL comparison is an error
"SC-N03" = "off"        # Don't enforce query naming convention
```

### Severity Levels

| Level | Behavior |
|-------|----------|
| `error` | Fails the lint/check command |
| `warn` | Reported but does not fail |
| `off` | Rule is disabled |

### Categories

| Category | Prefix | Description |
|----------|--------|-------------|
| `safety` | `SC-S` | Prevents dangerous operations |
| `naming` | `SC-N` | Enforces naming conventions |
| `style` | `SC-T` | Encourages clean SQL style |
| `performance` | `SC-P` | Flags performance issues |
| `antipattern` | `SC-A` | Catches common SQL mistakes |
| `codegen` | `SC-C` | Validates code generation annotations |

Category-level settings are overridden by rule-level settings.

## sqruff Configuration

Configure sqruff rules in `[lint.sqruff]`:

```toml
[lint.sqruff]
# Exclude specific rules
exclude_rules = ["LT01", "LT02", "CP01"]
```

### sqruff Rule Categories

| Prefix | Category | Description |
|--------|----------|-------------|
| AL | Aliasing | Table and column aliasing |
| AM | Ambiguous | Ambiguous SQL constructs |
| CP | Capitalisation | Keyword and identifier casing |
| CV | Convention | SQL conventions |
| LT | Layout | Formatting, spacing, indentation |
| RF | References | Column and table references |
| ST | Structure | SQL structure |

### Common sqruff Rules

| Code | Description |
|------|-------------|
| LT01 | Trailing whitespace |
| LT02 | Inconsistent indentation |
| LT05 | Line too long |
| LT12 | File must end with newline |
| CP01 | Keywords should be consistent case |
| AM01 | DISTINCT with GROUP BY |
| AM02 | UNION without DISTINCT/ALL |
| ST01 | Unnecessary ELSE NULL |

## Category-level Configuration

Override severity for all rules in a category:

```toml
[lint.categories]
safety = "error"
naming = "warn"
style = "off"
```

Per-rule overrides take precedence over category settings.

## Pre-commit Hook

Scythe provides a pre-commit hook for linting with auto-fix on commit. See [Pre-commit Hooks](pre-commit-hooks.md) for setup instructions.
