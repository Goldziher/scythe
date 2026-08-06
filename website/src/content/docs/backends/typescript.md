---
title: TypeScript
description: The typescript-postgres, typescript-pg, and typescript-kysely backends -- generated interfaces, queries, and type mappings.
---

Backends: `typescript-postgres` (postgres.js), `typescript-pg` (node-postgres), `typescript-kysely` (Kysely) | Engine: PostgreSQL (`typescript-kysely` also targets MySQL, SQLite, and MSSQL -- see [Kysely](#kysely) below)

All three backends share the same type mappings and TypeScript interfaces. They differ in query execution.

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

## Generated code -- shared interfaces

```typescript
export interface GetUserRow {
  id: number;
  name: string;
  email: string | null;
  createdAt: Date;
}

export interface ListUsersRow {
  id: number;
  name: string;
}
```

Note: field names use `camelCase` per the manifest naming convention.

## postgres.js

Uses tagged template literals for query parameterization.

### `:one`

```typescript
import postgres from "postgres";

export async function getUser(
  sql: postgres.Sql,
  id: number
): Promise<GetUserRow> {
  const [row] = await sql<GetUserRow[]>`
    SELECT id, name, email, created_at
    FROM users WHERE id = ${id}
  `;
  return row;
}
```

### `:many`

```typescript
export async function listUsers(
  sql: postgres.Sql,
  limit: number
): Promise<ListUsersRow[]> {
  return await sql<ListUsersRow[]>`
    SELECT id, name FROM users ORDER BY name LIMIT ${limit}
  `;
}
```

### `:exec`

```typescript
export async function createUser(
  sql: postgres.Sql,
  name: string,
  email: string | null
): Promise<void> {
  await sql`
    INSERT INTO users (name, email) VALUES (${name}, ${email})
  `;
}
```

## pg (node-postgres)

Uses `$N` positional parameters with `client.query()`.

### `:one`

```typescript
import { Client } from "pg";

export async function getUser(
  client: Client,
  id: number
): Promise<GetUserRow> {
  const { rows } = await client.query<GetUserRow>(
    "SELECT id, name, email, created_at FROM users WHERE id = $1",
    [id]
  );
  return rows[0];
}
```

### `:many`

```typescript
export async function listUsers(
  client: Client,
  limit: number
): Promise<ListUsersRow[]> {
  const { rows } = await client.query<ListUsersRow>(
    "SELECT id, name FROM users ORDER BY name LIMIT $1",
    [limit]
  );
  return rows;
}
```

### `:exec`

```typescript
export async function createUser(
  client: Client,
  name: string,
  email: string | null
): Promise<void> {
  await client.query(
    "INSERT INTO users (name, email) VALUES ($1, $2)",
    [name, email]
  );
}
```

## Kysely

`typescript-kysely` is dialect-parameterised, not driver-parameterised: generated functions take a `Kysely<DB>` handle and execute through Kysely's `sql` tagged-template. Kysely's own query compiler renders whatever placeholder syntax the connected `Dialect` needs at runtime, so the same generated call site works against every built-in Kysely dialect -- PostgreSQL, MySQL, SQLite, MSSQL -- and any third-party dialect scythe has never heard of, including `wasm-sqlite` and Node's built-in `node:sqlite` module.

```sql
-- @name GetUser
-- @returns :one
SELECT id, name, email, created_at FROM users WHERE id = ?;
```

(SQLite/MySQL source SQL uses bare `?`; PostgreSQL uses `$1`; MSSQL uses `@p1` -- the engine set in `scythe.toml` picks which syntax scythe expects, but none of it survives into the generated code, since every placeholder becomes a `${}` interpolation regardless.)

```typescript
import { Kysely, sql } from "kysely";

export async function getUser<DB = any>(
  db: Kysely<DB>,
  id: number
): Promise<GetUserRow | null> {
  const result = await sql<GetUserRow>`SELECT id, name, email, created_at FROM users WHERE id = ${id}`.execute(db);
  return result.rows[0] ?? null;
}
```

Pass any Kysely instance -- built-in or third-party dialect -- since the generated code never hardcodes a placeholder format:

```typescript
import { Kysely, SqliteDialect } from "kysely";
import Database from "better-sqlite3";
// or a third-party dialect, e.g. from kysely-sqlite-tools / wasm-sqlite / node:sqlite

const db = new Kysely<any>({
  dialect: new SqliteDialect({ database: new Database("app.db") }),
});

const user = await getUser(db, 1);
```

`:batch` queries run through Kysely's dialect-agnostic `db.transaction().execute(...)` API instead of hand-rolled `BEGIN`/`COMMIT`/`ROLLBACK` SQL text, so batches also work unmodified across every dialect.

### Outer-join precision (`outer_join_unions`)

A hand-written Kysely query has no way to express that a `LEFT JOIN`'s columns are null *together* -- Kysely infers result types from the query shape, not your schema's `NOT NULL` constraints, so every joined column just becomes independently optional. `typescript-kysely` supports the same opt-in `outer_join_unions` option as the other TypeScript backends: when a joined relation projects at least one `NOT NULL` column, scythe emits a discriminated union instead, ruling out states the query can never produce.

```sql
-- @name GetUserOrders
-- @returns :many
SELECT u.id, u.name, o.total, o.notes
FROM users u LEFT JOIN orders o ON u.id = o.user_id;
```

With `outer_join_unions = true` (and `orders.total NOT NULL`, `orders.notes` nullable):

```typescript
export type GetUserOrdersRow = {
  id: number;
  name: string;
} & (
  | { total: string; notes: string | null }
  | { total: null; notes: null }
);
```

### Options

| Option | Values | Default | Effect |
|--------|--------|---------|--------|
| `row_type` | `interface`, `zod` | `interface` | Emit plain TypeScript interfaces or Zod schemas + inferred types |
| `outer_join_unions` | `true`, `false` | `false` | Discriminated unions for outer-join nullability instead of independent optionals |

## Enum generation

```sql
CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');
```

```typescript
export enum UserStatus {
  Active = "active",
  Inactive = "inactive",
  Banned = "banned",
}
```

## Type mappings

| SQL Type | Neutral | TypeScript |
|----------|---------|------------|
| `SERIAL` / `INTEGER` | `int32` | `number` |
| `BIGINT` | `int64` | `number` |
| `TEXT` / `VARCHAR` | `string` | `string` |
| `BOOLEAN` | `bool` | `boolean` |
| `BYTEA` | `bytes` | `Buffer` |
| `UUID` | `uuid` | `string` |
| `NUMERIC` | `decimal` | `string` |
| `DATE` / `TIME` | `date` / `time` | `string` |
| `TIMESTAMPTZ` | `datetime_tz` | `Date` |
| `INTERVAL` | `interval` | `string` |
| `JSON` / `JSONB` | `json` | `Record<string, unknown>` |
| `TEXT[]` | `array<string>` | `string[]` |
| nullable column | `nullable` | `T \| null` |
