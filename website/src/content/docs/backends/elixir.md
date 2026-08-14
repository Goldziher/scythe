---
title: Elixir
description: The six Elixir backends -- generated modules, queries, and type mappings.
---

Backends: `elixir-postgrex`, `elixir-ecto`, `elixir-myxql`, `elixir-exqlite`, `elixir-tds`,
`elixir-jamdb` | Library: [Postgrex](https://hexdocs.pm/postgrex) / [Ecto](https://hexdocs.pm/ecto) /
[MyXQL](https://hexdocs.pm/myxql) / [Exqlite](https://hexdocs.pm/exqlite) /
[tds](https://hexdocs.pm/tds) / [jamdb_oracle](https://hex.pm/packages/jamdb_oracle)

`elixir-postgrex` supports PostgreSQL and Redshift. `elixir-ecto` supports PostgreSQL only, and
despite its name does **not** use `Ecto.Repo` or `Ecto.Adapters.SQL.query` -- there is not a single
reference to `Repo` or `Ecto.` anywhere in `elixir_ecto.rs`. It generates the same `Postgrex.query/3`
pattern as `elixir-postgrex`, taking a raw `conn` (not a `repo`) parameter. `elixir-myxql` supports
MySQL and MariaDB, `elixir-exqlite` supports SQLite only, `elixir-tds` supports MSSQL only, and
`elixir-jamdb` supports Oracle only -- see their sections below.

## SQL input

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

Schema:

```sql
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

## Postgrex

Backend: `elixir-postgrex` | Library: [Postgrex](https://hexdocs.pm/postgrex)

### Generated code

Row structs are **top-level, unqualified modules** (`GetUserRow`, not `Queries.GetUserRow`); the query
functions live together in a separate `Scythe.Queries` module. Postgrex uses the non-bang
`Postgrex.query/3` inside a `case`, returning a tagged tuple --
`{:ok, row} | {:error, :not_found} | {:error, term()}` -- not `Postgrex.query!/3` and not a bare struct
(`integration_tests/elixir-postgrex/generated/queries.ex:1-3,17-28,141-152`):

```elixir
# scythe:provenance v=0.15.0 backend=elixir-postgrex engine=postgresql schema=sch1:... queries=q1:...
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

## Ecto

Backend: `elixir-ecto` | Library: [Ecto](https://hexdocs.pm/ecto)

### Generated code

The only difference from `elixir-postgrex` is that everything -- row structs and query functions
alike -- is nested inside one `defmodule Scythe.Queries do ... end`, instead of row structs being
separate top-level modules
(`crates/scythe-codegen/src/backends/elixir_ecto.rs`;
`integration_tests/elixir-ecto/generated/queries.ex:1-3,17-41`):

```elixir
# scythe:provenance v=0.15.0 backend=elixir-ecto engine=postgresql schema=sch1:... queries=q1:...
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

## MyXQL

Backend: `elixir-myxql` | Library: [MyXQL](https://hexdocs.pm/myxql) | Engines: MySQL, MariaDB

Same layout as `elixir-postgrex`: row structs are top-level, unqualified modules; query functions live
together in a separate `Scythe.Queries` module (`integration_tests/elixir-myxql/generated/queries.ex`).
Query functions call `MyXQL.query/3` and match on `%MyXQL.Result{rows: ...}`, returning
`{:ok, row} | {:error, :not_found} | {:error, term()}` for `:one`/`:opt` queries, the same tagged-tuple
shape as `elixir-postgrex` (`crates/scythe-codegen/src/backends/elixir_myxql.rs`).

## Exqlite

Backend: `elixir-exqlite` | Library: [Exqlite](https://hexdocs.pm/exqlite) | Engine: SQLite

Same top-level-modules-plus-`Scythe.Queries` layout as `elixir-postgrex` and `elixir-myxql`
(`integration_tests/elixir-exqlite/generated/queries.ex`). Unlike those two, query functions do not
call a single `query/3` -- Exqlite has no such call. Instead each function drives the low-level
`Exqlite.Sqlite3` NIF API directly through an Elixir `with` chain: `prepare/2`, `bind/2`,
`step/2`/`fetch_all/2`, then `release/2` (`crates/scythe-codegen/src/backends/elixir_exqlite.rs`).

## TDS

Backend: `elixir-tds` | Library: [tds](https://hexdocs.pm/tds) | Engine: MSSQL

Query functions call `Tds.query/3`. Parameters are not passed as bare positional values but as
`%Tds.Parameter{name: "@1", value: ..., type: :atom}` structs, with the type atom (`:integer`,
`:string`, `:decimal`, `:datetime`, `:boolean`, ...) derived from the neutral type
(`crates/scythe-codegen/src/backends/elixir_tds.rs`). Boolean params are coerced to `1`/`0` before
encoding, since tds's `:boolean` encoder accepts only integers or bitstrings, not Elixir booleans --
`nil` is passed through unchanged so a NULL boolean stays SQL `NULL` rather than becoming `false`.

## Jamdb Oracle

Backend: `elixir-jamdb` | Library: [jamdb_oracle](https://hex.pm/packages/jamdb_oracle) (alias:
`jamdb`) | Engine: Oracle

Query functions call `Jamdb.Oracle.query/3`. Placeholders are rewritten from `$1`, `$2`, ... to
Oracle-style `:1`, `:2`, ... bind variables before the SQL string is emitted
(`crates/scythe-codegen/src/backends/elixir_jamdb.rs`).
