---
title: Ruby
description: The six Ruby backends -- generated queries and type mappings.
---

Backends: `ruby-pg`, `ruby-trilogy`, `ruby-mysql2`, `ruby-sqlite3`, `ruby-tiny-tds`, `ruby-oci8` |
Library: [pg gem](https://github.com/ged/ruby-pg) /
[Trilogy](https://github.com/trilogy-libraries/trilogy) /
[mysql2](https://github.com/brianmario/mysql2) / [sqlite3](https://github.com/sparklemotion/sqlite3-ruby)
/ [tiny_tds](https://github.com/rails-sqlserver/tiny_tds) / [ruby-oci8](https://github.com/kubo/ruby-oci8)

`ruby-pg` targets PostgreSQL and Redshift through libpq bind parameters. `ruby-trilogy` targets MySQL
through GitHub's Trilogy client, which has no bind-parameter API at all -- see [Trilogy](#trilogy)
below. `ruby-mysql2` targets MySQL and MariaDB, `ruby-sqlite3` targets SQLite, `ruby-tiny-tds` targets
MSSQL, and `ruby-oci8` targets Oracle -- see their sections below.

## Ruby 3.4+ and BigDecimal

`bigdecimal` stopped shipping as a default gem in Ruby 3.4.0. `ruby-pg`, `ruby-mysql2`, and
`ruby-trilogy` emit `require "bigdecimal/util"` whenever a query's generated code applies the
`.to_d` coercion for a `decimal` column (`crates/scythe-codegen/src/backends/ruby_rbs.rs`,
`ruby_generated_code_needs_bigdecimal_util`); on Ruby 3.4+ that `require` raises `LoadError`
unless something in the bundle depends on the `bigdecimal` gem. Add it to your project's
`Gemfile` alongside the driver gem:

```ruby
gem "pg"        # or "mysql2", "trilogy"
gem "bigdecimal"
```

`ruby-sqlite3`, `ruby-tiny-tds`, and `ruby-oci8` never emit that `require` and are unaffected.

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
# scythe:provenance v=0.15.0 backend=ruby-pg engine=postgresql schema=sch1:... queries=q1:...

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
# scythe:provenance v=0.15.0 backend=ruby-trilogy engine=mysql schema=sch1:... queries=q1:...

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

## mysql2

Backend: `ruby-mysql2` | Library: [mysql2](https://github.com/brianmario/mysql2) | Engines: MySQL,
MariaDB

Everything sits inside a single `module Queries ... end`, matching `ruby-pg`
(`integration_tests/ruby-mysql2/generated/queries.rb`). Unlike `ruby-trilogy`, `mysql2` has a real
bind-parameter API: query functions call `client.prepare(sql)` then `stmt.execute(*args)` with
positional `?` placeholders, not string interpolation
(`crates/scythe-codegen/src/backends/ruby_mysql2.rs`).

## sqlite3

Backend: `ruby-sqlite3` | Library: [sqlite3](https://github.com/sparklemotion/sqlite3-ruby) | Engine:
SQLite

Same single-`module Queries` shape as `ruby-pg` and `ruby-mysql2`
(`integration_tests/ruby-sqlite3/generated/queries.rb`). Query functions call `db.execute(sql, [...])`
directly (`:many`) or `db.get_first_row(sql, [...])` (`:one`/`:opt`), with positional `?`
placeholders (`crates/scythe-codegen/src/backends/ruby_sqlite3.rs`).

## tiny_tds

Backend: `ruby-tiny-tds` | Library: [tiny_tds](https://github.com/rails-sqlserver/tiny_tds) | Engine:
MSSQL

Like `ruby-trilogy`, TinyTDS has **no bind-parameter API**, so values are interpolated directly into
the SQL string rather than passed to `client.execute`. Numeric and boolean params are interpolated
bare (booleans as `1`/`0`); string params are escaped with `client.escape(...)` and quoted, with
`NULL` substituted for nil values on nullable columns
(`crates/scythe-codegen/src/backends/ruby_tiny_tds.rs`).

## oci8

Backend: `ruby-oci8` | Library: [ruby-oci8](https://github.com/kubo/ruby-oci8) | Engine: Oracle

Everything sits inside a single `module Queries ... end`. Query functions call `conn.exec(sql, ...)`
with genuine positional bind parameters, rewritten from `$1`, `$2`, ... to Oracle-style `:1`, `:2`,
... placeholders. `RETURNING` clauses use an explicit `conn.parse` / `cursor.bind_param` /
`cursor.exec` sequence with output binds instead of `conn.exec`
(`crates/scythe-codegen/src/backends/ruby_oci8.rs`).
| `nullable` | `T` (no wrapper; Ruby is dynamically typed) |
