---
title: Ruby
description: The ruby-pg and ruby-trilogy backends -- generated queries and type mappings.
---

Backends: `ruby-pg`, `ruby-trilogy` | Library: [pg gem](https://github.com/ged/ruby-pg) /
[Trilogy](https://github.com/trilogy-libraries/trilogy)

`ruby-pg` targets PostgreSQL through libpq bind parameters. `ruby-trilogy` targets MySQL through
GitHub's Trilogy client, which has no bind-parameter API at all -- see [Trilogy](#trilogy) below.

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

## pg

Backend: `ruby-pg` | Library: [pg gem](https://github.com/ged/ruby-pg)

### Generated code

Parameters are **positional**, not keyword arguments -- `def self.get_user(conn, id)`, not
`def self.get_user(conn, id:)`. A call site written against keyword arguments raises `ArgumentError`.
`:one` guards on `result.ntuples.zero?` and returns the raw string from `pg` with no `Time.parse`
wrapper. Everything sits inside a single `module Queries ... end`
(`integration_tests/ruby-pg/generated/queries.rb:1-21`):

```ruby
# scythe:provenance v=0.14.0 backend=ruby-pg engine=postgresql schema=sch1:... queries=q1:...

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

## Trilogy

Backend: `ruby-trilogy` | Library: [Trilogy](https://github.com/trilogy-libraries/trilogy) (MySQL driver)

Trilogy is GitHub's MySQL client library for Ruby. `ruby-trilogy` has **no bind-parameter API** --
there is no `?`/`$N` placeholder anywhere in the generated SQL. Instead, values are interpolated
directly into the SQL string; string and enum values are escaped with `client.escape(...)` and
quoted, numeric values are interpolated bare
(`crates/scythe-codegen/src/backends/ruby_trilogy.rs`;
`integration_tests/ruby-trilogy/generated/queries.rb:15-38`):

```ruby
# frozen_string_literal: true
# scythe:provenance v=0.14.0 backend=ruby-trilogy engine=mysql schema=sch1:... queries=q1:...

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
