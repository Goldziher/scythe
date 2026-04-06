# TypeScript

Backends: `typescript-postgres` (postgres.js), `typescript-pg` (node-postgres) | Engine: PostgreSQL

Both backends share the same type mappings and TypeScript interfaces. They differ in query execution.

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
