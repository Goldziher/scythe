# Other backends

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

```elixir
defmodule Queries.GetUserRow do
  defstruct [:id, :name, :email, :created_at]

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    created_at: DateTime.t()
  }
end

def get_user(conn, id) do
  %Postgrex.Result{rows: [row]} =
    Postgrex.query!(conn,
      "SELECT id, name, email, created_at FROM users WHERE id = $1",
      [id]
    )
  %Queries.GetUserRow{
    id: Enum.at(row, 0),
    name: Enum.at(row, 1),
    email: Enum.at(row, 2),
    created_at: Enum.at(row, 3)
  }
end

def list_users(conn, limit) do
  %Postgrex.Result{rows: rows} =
    Postgrex.query!(conn,
      "SELECT id, name FROM users ORDER BY name LIMIT $1",
      [limit]
    )
  Enum.map(rows, fn row ->
    %Queries.ListUsersRow{id: Enum.at(row, 0), name: Enum.at(row, 1)}
  end)
end

def create_user(conn, name, email) do
  Postgrex.query!(conn,
    "INSERT INTO users (name, email) VALUES ($1, $2)",
    [name, email]
  )
  :ok
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

```ruby
GetUserRow = Data.define(:id, :name, :email, :created_at)

def self.get_user(conn, id:)
  result = conn.exec_params(
    "SELECT id, name, email, created_at FROM users WHERE id = $1",
    [id]
  )
  row = result.first
  GetUserRow.new(
    id: row["id"].to_i,
    name: row["name"],
    email: row["email"],
    created_at: Time.parse(row["created_at"])
  )
end

ListUsersRow = Data.define(:id, :name)

def self.list_users(conn, limit:)
  result = conn.exec_params(
    "SELECT id, name FROM users ORDER BY name LIMIT $1",
    [limit]
  )
  result.map { |row| ListUsersRow.new(id: row["id"].to_i, name: row["name"]) }
end

def self.create_user(conn, name:, email:)
  conn.exec_params(
    "INSERT INTO users (name, email) VALUES ($1, $2)",
    [name, email]
  )
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

```php
readonly class GetUserRow
{
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public \DateTimeImmutable $createdAt,
    ) {}
}

function getUser(PDO $db, int $id): GetUserRow
{
    $stmt = $db->prepare(
        'SELECT id, name, email, created_at FROM users WHERE id = :p1'
    );
    $stmt->execute(['p1' => $id]);
    $row = $stmt->fetch(PDO::FETCH_ASSOC);

    return new GetUserRow(
        id: (int) $row['id'],
        name: $row['name'],
        email: $row['email'],
        createdAt: new \DateTimeImmutable($row['created_at']),
    );
}

function createUser(PDO $db, string $name, ?string $email): void
{
    $stmt = $db->prepare(
        'INSERT INTO users (name, email) VALUES (:p1, :p2)'
    );
    $stmt->execute(['p1' => $name, 'p2' => $email]);
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

Trilogy is GitHub's MySQL client library for Ruby. It uses array-based row access for performance.

### Generated code

```ruby
# frozen_string_literal: true

GetUserRow = Data.define(:id, :name, :email, :created_at)

def self.get_user(client, id:)
  result = client.query_with_flags(
    "SELECT id, name, email, created_at FROM users WHERE id = ?",
    [id],
    Trilogy::QUERY_FLAGS_CAST
  )
  row = result.rows.first
  return nil if row.nil?
  GetUserRow.new(
    id: row[0],
    name: row[1],
    email: row[2],
    created_at: row[3]
  )
end

ListUsersRow = Data.define(:id, :name)

def self.list_users(client, limit:)
  result = client.query_with_flags(
    "SELECT id, name FROM users ORDER BY name LIMIT ?",
    [limit],
    Trilogy::QUERY_FLAGS_CAST
  )
  result.rows.map { |row| ListUsersRow.new(id: row[0], name: row[1]) }
end

def self.create_user(client, name:, email:)
  client.query(
    "INSERT INTO users (name, email) VALUES (?, ?)",
    [name, email]
  )
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

Backend: `elixir-ecto` | Library: [Ecto](https://hexdocs.pm/ecto) (Repo-based)

Uses `Ecto.Adapters.SQL.query` for raw SQL execution through an Ecto Repo.

### Generated code

```elixir
defmodule Queries.GetUserRow do
  defstruct [:id, :name, :email, :created_at]

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    created_at: DateTime.t()
  }
end

@spec get_user(Ecto.Repo.t(), integer()) ::
  {:ok, %Queries.GetUserRow{}} | {:error, term()}
def get_user(repo, id) do
  case Ecto.Adapters.SQL.query(repo,
    "SELECT id, name, email, created_at FROM users WHERE id = $1",
    [id]) do
    {:ok, %{rows: [row], columns: _columns}} ->
      [id, name, email, created_at] = row
      {:ok, %Queries.GetUserRow{
        id: id, name: name, email: email,
        created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec list_users(Ecto.Repo.t(), integer()) ::
  {:ok, [%Queries.ListUsersRow{}]} | {:error, term()}
def list_users(repo, limit) do
  case Ecto.Adapters.SQL.query(repo,
    "SELECT id, name FROM users ORDER BY name LIMIT $1",
    [limit]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn [id, name] ->
        %Queries.ListUsersRow{id: id, name: name}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec create_user(Ecto.Repo.t(), String.t(), String.t() | nil) ::
  :ok | {:error, term()}
def create_user(repo, name, email) do
  case Ecto.Adapters.SQL.query(repo,
    "INSERT INTO users (name, email) VALUES ($1, $2)",
    [name, email]) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
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

Uses `Amp\Sql\SqlConnectionPool` for async database access with AMPHP's event loop.

### Generated code

```php
<?php

declare(strict_types=1);

use Amp\Sql\SqlConnectionPool;

readonly class GetUserRow
{
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public \DateTimeImmutable $createdAt,
    ) {}
}

function getUser(SqlConnectionPool $pool, int $id): ?GetUserRow
{
    $result = $pool->prepare(
        'SELECT id, name, email, created_at FROM users WHERE id = ?'
    )->execute([$id]);

    foreach ($result as $row) {
        return new GetUserRow(
            id: (int) $row['id'],
            name: $row['name'],
            email: $row['email'],
            createdAt: new \DateTimeImmutable($row['created_at']),
        );
    }

    return null;
}

/** @return list<ListUsersRow> */
function listUsers(SqlConnectionPool $pool, int $limit): array
{
    $result = $pool->prepare(
        'SELECT id, name FROM users ORDER BY name LIMIT ?'
    )->execute([$limit]);

    $rows = [];
    foreach ($result as $row) {
        $rows[] = new ListUsersRow(
            id: (int) $row['id'],
            name: $row['name'],
        );
    }

    return $rows;
}

function createUser(
    SqlConnectionPool $pool, string $name, ?string $email
): void {
    $pool->prepare(
        'INSERT INTO users (name, email) VALUES (?, ?)'
    )->execute([$name, $email]);
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
