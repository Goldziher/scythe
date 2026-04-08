# Scythe

**Polyglot SQL-to-code generator with built-in linting and formatting.**

Write SQL. Get type-safe code. In any language.

## Why Scythe

- **10 languages, 27 backend drivers** -- Rust, Python, TypeScript, Go, Java, Kotlin, C#, Elixir, Ruby, PHP
- **3 databases** -- PostgreSQL, MySQL, SQLite -- all 10 languages supported on every engine
- **93 lint rules (22 custom + 71 sqruff)** -- catch bugs before they ship
- **SQL formatting** -- via sqruff integration
- **Smart type inference** -- nullability from JOINs, COALESCE, window functions, aggregates
- **Configurable row types** -- Pydantic, msgspec, Zod, or language defaults per backend
- **`@optional` parameters** -- SQL rewriting for conditional filters without dynamic query building

## Quick Install

```bash
cargo install scythe-cli
# or
brew install Goldziher/tap/scythe
```

## 30-Second Example

```sql
-- @name GetUserOrders
-- @returns :many
SELECT u.id, u.name, o.total, o.notes
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1;
```

Scythe knows `o.total` and `o.notes` are nullable (right side of LEFT JOIN) and generates type-safe code:

=== "Rust"

    ```rust
    pub struct GetUserOrdersRow {
        pub id: i32,
        pub name: String,
        pub total: Option<rust_decimal::Decimal>,
        pub notes: Option<String>,
    }
    ```

=== "Python"

    ```python
    @dataclass
    class GetUserOrdersRow:
        id: int
        name: str
        total: decimal.Decimal | None
        notes: str | None
    ```

=== "TypeScript"

    ```typescript
    interface GetUserOrdersRow {
        id: number;
        name: string;
        total: string | null;
        notes: string | null;
    }
    ```

=== "Go"

    ```go
    type GetUserOrdersRow struct {
        ID    int32    `json:"id"`
        Name  string   `json:"name"`
        Total *string  `json:"total"`
        Notes *string  `json:"notes"`
    }
    ```

=== "Java"

    ```java
    public record GetUserOrdersRow(
        int id,
        String name,
        @Nullable BigDecimal total,
        @Nullable String notes
    ) {}
    ```

=== "Kotlin"

    ```kotlin
    data class GetUserOrdersRow(
        val id: Int,
        val name: String,
        val total: java.math.BigDecimal?,
        val notes: String?,
    )
    ```

=== "C#"

    ```csharp
    public record GetUserOrdersRow(
        int Id,
        string Name,
        decimal? Total,
        string? Notes
    );
    ```

=== "Elixir"

    ```elixir
    defmodule GetUserOrdersRow do
      @type t :: %__MODULE__{
        id: integer(),
        name: String.t(),
        total: Decimal.t() | nil,
        notes: String.t() | nil
      }
      defstruct [:id, :name, :total, :notes]
    end
    ```

=== "Ruby"

    ```ruby
    module Queries
      GetUserOrdersRow = Data.define(
        :id, :name, :total, :notes
      )
    end
    ```

=== "PHP"

    ```php
    readonly class GetUserOrdersRow {
        public function __construct(
            public int $id,
            public string $name,
            public ?string $total,
            public ?string $notes,
        ) {}
    }
    ```

## Learn More

- [Quickstart](getting-started/quickstart.md) -- from zero to generated code in 5 minutes
- [Philosophy](philosophy.md) -- why compile SQL instead of using an ORM
- [Alternatives](comparisons/alternatives.md) -- how scythe compares to sqlc, SQLDelight, jOOQ, and ORMs
