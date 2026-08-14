---
title: PHP
description: The php-pdo and php-amphp backends -- generated classes, queries, and type mappings.
---

Backends: `php-pdo`, `php-amphp` | Library: PDO / [AMPHP SQL](https://github.com/amphp/sql) (async)

`php-pdo` is synchronous, built on PDO. `php-amphp` is structurally identical but async, built on
`Amp\Sql\SqlConnectionPool` with AMPHP's event loop.

`php-pdo` supports PostgreSQL, MySQL, MariaDB, SQLite, MSSQL, Redshift, and Snowflake (no Oracle,
despite a `php-pdo.oracle.toml` manifest existing in the tree). `php-amphp` supports PostgreSQL,
MySQL, and MariaDB only -- pick `php-pdo` for SQLite, MSSQL, Redshift, or Snowflake.

Both share the same generated-code shape:
`snake_case` properties by default (matching the SQL column name), query functions as `public static`
methods on a single `final class Queries`, a generated `public static function fromRow(array $row): self`
per row type, and a `namespace App\Generated;` default that both backends accept an undocumented
`namespace` option to change or clear (`crates/scythe-codegen/src/backends/php_pdo.rs`).

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

## PDO

Backend: `php-pdo` | Library: PDO

### Generated code

`:many` returns **`\Generator`** and `yield`s rows rather than building an array
(`integration_tests/php-pdo/generated/queries.php:1-9,198-225`):

```php
<?php
// scythe:provenance v=0.15.0 backend=php-pdo engine=postgresql schema=sch1:... queries=q1:...

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

## AMPHP

Backend: `php-amphp` | Library: [AMPHP SQL](https://github.com/amphp/sql) (async)

### Generated code

Structurally identical to `php-pdo` except the driver is
`SqlConnectionPool`/`->prepare(...)->execute([...])` instead of `\PDO`, and placeholders are bare `?`
instead of `:p1`. `:many` also returns **`\Generator`**, `yield`ing rows rather than building an array
(`integration_tests/php-amphp/generated/queries.php:1-9,198-225`):

```php
<?php
// scythe:provenance v=0.15.0 backend=php-amphp engine=postgresql schema=sch1:... queries=q1:...

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
