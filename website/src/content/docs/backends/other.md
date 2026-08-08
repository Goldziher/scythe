---
title: Other backends
description: Elixir, Ruby, and PHP backends -- Postgrex, pg, PDO, Trilogy, Ecto, and AMPHP.
---

## Elixir + Postgrex

Backend: `elixir-postgrex` | Library: [Postgrex](https://hexdocs.pm/postgrex)

```sql
-- @name GetUser
-- @returns :one
SELECT id, name, email, created_at FROM users WHERE id = $1;

-- @name ListUsers
-- @returns :many
SELECT id, name FROM users ORDER BY name LIMIT $1;

-- @name CreateUser
-- @returns :exec
INSERT INTO users (name, email) VALUES ($1, $2);
```

### Generated code

Row structs are **top-level, unqualified modules** (`GetUserRow`, not `Queries.GetUserRow`); the query
functions live together in a separate `Scythe.Queries` module. Postgrex uses the non-bang
`Postgrex.query/3` inside a `case`, returning a tagged tuple --
`{:ok, row} | {:error, :not_found} | {:error, term()}` -- not `Postgrex.query!/3` and not a bare struct
(`integration_tests/elixir-postgrex/generated/queries.ex:1-3,17-28,141-152`):

```elixir
# scythe:provenance v=0.13.0 backend=elixir-postgrex engine=postgresql schema=sch1:...
defmodule GetUserRow do
  @moduledoc "Row type for GetUser queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    created_at: DateTime.t()
  }
  defstruct [:id, :name, :email, :created_at]
end

defmodule ListUsersRow do
  @moduledoc "Row type for ListUsers queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t()
  }
  defstruct [:id, :name]
end

defmodule Scythe.Queries do

@spec get_user(Postgrex.conn(), integer()) :: {:ok, %GetUserRow{}} | {:error, :not_found} | {:error, term()}
def get_user(conn, id) do
  case Postgrex.query(conn, "SELECT id, name, email, created_at FROM users WHERE id = $1", [id]) do
    {:ok, %{rows: [row | _]}} ->
      [id, name, email, created_at] = row
      {:ok, %GetUserRow{id: id, name: name, email: email, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec list_users(Postgrex.conn(), integer()) :: {:ok, [%ListUsersRow{}]} | {:error, term()}
def list_users(conn, limit) do
  case Postgrex.query(conn, "SELECT id, name FROM users ORDER BY name LIMIT $1", [limit]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name] = row
        %ListUsersRow{id: id, name: name}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec create_user(Postgrex.conn(), String.t(), String.t() | nil) :: :ok | {:error, term()}
def create_user(conn, name, email) do
  case Postgrex.query(conn, "INSERT INTO users (name, email) VALUES ($1, $2)", [name, email]) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

end
```

### Key types

| Neutral | Elixir |
|---------|--------|
| `int32` | `integer()` |
| `string` | `String.t()` |
| `datetime_tz` | `DateTime.t()` |
| `uuid` | `String.t()` |
| `json` | `map()` |
| `nullable` | `T \| nil` |

---

## Ruby + pg

Backend: `ruby-pg` | Library: [pg gem](https://github.com/ged/ruby-pg)

### Generated code

Parameters are **positional**, not keyword arguments -- `def self.get_user(conn, id)`, not
`def self.get_user(conn, id:)`. A call site written against keyword arguments raises `ArgumentError`.
`:one` guards on `result.ntuples.zero?` and returns the raw string from `pg` with no `Time.parse`
wrapper. Everything sits inside a single `module Queries ... end`
(`integration_tests/ruby-pg/generated/queries.rb:1-21`):

```ruby
# scythe:provenance v=0.13.0 backend=ruby-pg engine=postgresql schema=sch1:...

module Queries

  GetUserRow = Data.define(:id, :name, :email, :created_at)

  def self.get_user(conn, id)
    result = conn.exec_params("SELECT id, name, email, created_at FROM users WHERE id = $1", [id])
    return nil if result.ntuples.zero?
    row = result[0]
    GetUserRow.new(id: row["id"].to_i, name: row["name"], email: row["email"], created_at: row["created_at"])
  end

  ListUsersRow = Data.define(:id, :name)

  def self.list_users(conn, limit)
    result = conn.exec_params("SELECT id, name FROM users ORDER BY name LIMIT $1", [limit])
    result.map { |row| ListUsersRow.new(id: row["id"].to_i, name: row["name"]) }
  end

  def self.create_user(conn, name, email)
    conn.exec_params("INSERT INTO users (name, email) VALUES ($1, $2)", [name, email])
    nil
  end

end
```

### Key types

| Neutral | Ruby |
|---------|------|
| `int32` | `Integer` |
| `string` | `String` |
| `datetime_tz` | `Time` |
| `uuid` | `String` |
| `decimal` | `BigDecimal` |
| `json` | `Hash` |
| `nullable` | `T` (no wrapper; Ruby is dynamically typed) |

---

## PHP + PDO

Backend: `php-pdo` | Library: PDO

### Generated code

Row properties are `snake_case` by default (matching the SQL column name); query functions are
`public static` methods on a single `final class Queries`, typed against `\PDO`. Each row type gets a
generated `public static function fromRow(array $row): self`. `:many` returns **`\Generator`** and
`yield`s rows rather than building an array. Files default to `namespace App\Generated;` -- both
`php-pdo` and `php-amphp` accept an undocumented `namespace` option to change or clear it
(`crates/scythe-codegen/src/backends/php_pdo.rs:22,44,188-193`;
`integration_tests/php-pdo/generated/queries.php:1-9,198-225`):

```php
<?php
// scythe:provenance v=0.13.0 backend=php-pdo engine=postgresql schema=sch1:...

declare(strict_types=1);

namespace App\Generated;

readonly class GetUserRow
{
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self
    {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
            created_at: new \DateTimeImmutable($row['created_at']),
        );
    }
}

final class Queries
{
    public static function getUser(\PDO $pdo, int $id): ?GetUserRow
    {
        $stmt = $pdo->prepare("SELECT id, name, email, created_at FROM users WHERE id = :p1");
        $stmt->execute(["p1" => $id]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        return $row ? GetUserRow::fromRow($row) : null;
    }

    public static function listUsers(\PDO $pdo, int $limit): \Generator
    {
        $stmt = $pdo->prepare("SELECT id, name FROM users ORDER BY name LIMIT :p1");
        $stmt->execute(["p1" => $limit]);
        while ($row = $stmt->fetch(\PDO::FETCH_ASSOC)) {
            yield ListUsersRow::fromRow($row);
        }
    }

    public static function createUser(\PDO $pdo, string $name, ?string $email): void
    {
        $stmt = $pdo->prepare("INSERT INTO users (name, email) VALUES (:p1, :p2)");
        $stmt->execute(["p1" => $name, "p2" => $email]);
    }
}
```

### Key types

| Neutral | PHP |
|---------|-----|
| `int32` | `int` |
| `string` | `string` |
| `datetime_tz` | `\DateTimeImmutable` |
| `uuid` | `string` |
| `decimal` | `string` |
| `json` | `array` |
| `nullable` | `?T` |

---

## Ruby + Trilogy

Backend: `ruby-trilogy` | Library: [Trilogy](https://github.com/trilogy-libraries/trilogy) (MySQL driver)

Trilogy is GitHub's MySQL client library for Ruby. `ruby-trilogy` has **no bind-parameter API** --
there is no `?`/`$N` placeholder anywhere in the generated SQL. Instead, values are interpolated
directly into the SQL string; string and enum values are escaped with `client.escape(...)` and
quoted, numeric values are interpolated bare
(`crates/scythe-codegen/src/backends/ruby_trilogy.rs:56-64,77`;
`integration_tests/ruby-trilogy/generated/queries.rb:15-38`):

```ruby
# frozen_string_literal: true
# scythe:provenance v=0.13.0 backend=ruby-trilogy engine=mysql schema=sch1:...

module Queries

  GetUserRow = Data.define(:id, :name, :email, :created_at)

  def self.get_user(client, id)
    results = client.query("SELECT id, name, email, created_at FROM users WHERE id = #{id}")
    row = results.first
    return nil if row.nil?
    GetUserRow.new(id: row[0].to_i, name: row[1], email: row[2], created_at: row[3])
  end

  ListUsersRow = Data.define(:id, :name)

  def self.list_users(client, limit)
    results = client.query("SELECT id, name FROM users ORDER BY name LIMIT #{limit}")
    results.map { |row| ListUsersRow.new(id: row[0].to_i, name: row[1]) }
  end

  def self.create_user(client, name, email)
    client.query("INSERT INTO users (name, email) VALUES ('#{client.escape(name.to_s)}', '#{client.escape(email.to_s)}')")
    nil
  end

end
```

### Key types

| Neutral | Ruby (Trilogy) |
|---------|----------------|
| `int32` | `Integer` |
| `string` | `String` |
| `datetime_tz` | `Time` |
| `uuid` | `String` |
| `decimal` | `BigDecimal` |
| `json` | `Hash` |
| `nullable` | `T` (no wrapper; Ruby is dynamically typed) |

---

## Elixir + Ecto

Backend: `elixir-ecto` | Library: [Ecto](https://hexdocs.pm/ecto)

`elixir-ecto` does **not** use `Ecto.Repo` or `Ecto.Adapters.SQL.query` -- there is not a single
reference to `Repo` or `Ecto.` anywhere in `elixir_ecto.rs`. It generates the same
`Postgrex.query/3` + tagged-tuple pattern as `elixir-postgrex`, taking a raw `conn` (not a `repo`)
parameter; the only difference from `elixir-postgrex` is that everything -- row structs and query
functions alike -- is nested inside one `defmodule Scythe.Queries do ... end`, instead of row structs
being separate top-level modules
(`crates/scythe-codegen/src/backends/elixir_ecto.rs`;
`integration_tests/elixir-ecto/generated/queries.ex:1-3,17-41`):

```elixir
# scythe:provenance v=0.13.0 backend=elixir-ecto engine=postgresql schema=sch1:...
defmodule Scythe.Queries do

defmodule GetUserRow do
  @moduledoc "Row type for GetUser queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    created_at: DateTime.t()
  }
  defstruct [:id, :name, :email, :created_at]
end

@spec get_user(Postgrex.conn(), integer()) :: {:ok, %GetUserRow{}} | {:error, :not_found} | {:error, term()}
def get_user(conn, id) do
  case Postgrex.query(conn, "SELECT id, name, email, created_at FROM users WHERE id = $1", [id]) do
    {:ok, %{rows: [row | _]}} ->
      [id, name, email, created_at] = row
      {:ok, %GetUserRow{id: id, name: name, email: email, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

defmodule ListUsersRow do
  @moduledoc "Row type for ListUsers queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t()
  }
  defstruct [:id, :name]
end

@spec list_users(Postgrex.conn(), integer()) :: {:ok, [%ListUsersRow{}]} | {:error, term()}
def list_users(conn, limit) do
  case Postgrex.query(conn, "SELECT id, name FROM users ORDER BY name LIMIT $1", [limit]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name] = row
        %ListUsersRow{id: id, name: name}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec create_user(Postgrex.conn(), String.t(), String.t() | nil) :: :ok | {:error, term()}
def create_user(conn, name, email) do
  case Postgrex.query(conn, "INSERT INTO users (name, email) VALUES ($1, $2)", [name, email]) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

end
```

### Key types

| Neutral | Elixir (Ecto) |
|---------|---------------|
| `int32` | `integer()` |
| `string` | `String.t()` |
| `datetime_tz` | `DateTime.t()` |
| `uuid` | `String.t()` |
| `json` | `map()` |
| `nullable` | `T \| nil` |

---

## PHP + AMPHP

Backend: `php-amphp` | Library: [AMPHP SQL](https://github.com/amphp/sql) (async)

Uses `Amp\Sql\SqlConnectionPool` for async database access with AMPHP's event loop. Structurally
identical to `php-pdo` -- `snake_case` properties, a generated `fromRow`, `public static` methods on
`final class Queries`, the same `namespace App\Generated;` default and `namespace` option -- except the
driver is `SqlConnectionPool`/`->prepare(...)->execute([...])` instead of `\PDO`, and placeholders are
bare `?` instead of `:p1`. `:many` also returns **`\Generator`**, `yield`ing rows rather than
building an array (`integration_tests/php-amphp/generated/queries.php:1-9,198-225`):

```php
<?php
// scythe:provenance v=0.13.0 backend=php-amphp engine=postgresql schema=sch1:...

declare(strict_types=1);

namespace App\Generated;

readonly class GetUserRow
{
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self
    {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
            created_at: new \DateTimeImmutable($row['created_at']),
        );
    }
}

final class Queries
{
    public static function getUser(\Amp\Sql\SqlConnectionPool $pool, int $id): ?GetUserRow
    {
        $result = $pool->prepare("SELECT id, name, email, created_at FROM users WHERE id = ?")->execute([$id]);
        foreach ($result as $row) {
            return GetUserRow::fromRow($row);
        }
        return null;
    }

    public static function listUsers(\Amp\Sql\SqlConnectionPool $pool, int $limit): \Generator
    {
        $result = $pool->prepare("SELECT id, name FROM users ORDER BY name LIMIT ?")->execute([$limit]);
        foreach ($result as $row) {
            yield ListUsersRow::fromRow($row);
        }
    }

    public static function createUser(\Amp\Sql\SqlConnectionPool $pool, string $name, ?string $email): void
    {
        $pool->prepare("INSERT INTO users (name, email) VALUES (?, ?)")->execute([$name, $email]);
    }
}
```

### Key types

| Neutral | PHP (AMPHP) |
|---------|-------------|
| `int32` | `int` |
| `string` | `string` |
| `datetime_tz` | `\DateTimeImmutable` |
| `uuid` | `string` |
| `decimal` | `string` |
| `json` | `array` |
| `nullable` | `?T` |
