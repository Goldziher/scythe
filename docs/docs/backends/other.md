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
        'SELECT id, name, email, created_at FROM users WHERE id = $1'
    );
    $stmt->execute([$id]);
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
        'INSERT INTO users (name, email) VALUES ($1, $2)'
    );
    $stmt->execute([$name, $email]);
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
