# SQLite

Scythe supports SQLite with its simplified type affinity system across all 10 languages. SQLite support operates at the parser and analyzer level -- SQL parsing, type inference, and nullability analysis are fully SQLite-aware. The code generation backends work the same regardless of the source database.

## Backend support

Every language has at least one SQLite backend. Multi-engine backends (like `java-jdbc`, `php-pdo`, `rust-sqlx`) load engine-specific manifests automatically.

| Language | Backend | Library |
|----------|---------|---------|
| Rust | `rust-sqlx` | sqlx |
| Python | `python-aiosqlite` | aiosqlite |
| TypeScript | `typescript-better-sqlite3` | better-sqlite3 |
| Go | `go-database-sql` | database/sql |
| Java | `java-jdbc` | JDBC |
| Kotlin | `kotlin-jdbc` | JDBC |
| C# | `csharp-microsoft-sqlite` | Microsoft.Data.Sqlite |
| Elixir | `elixir-exqlite` | Exqlite |
| Ruby | `ruby-sqlite3` | sqlite3 gem |
| PHP | `php-pdo` | PDO |

## Type affinity system

SQLite uses [type affinity](https://www.sqlite.org/datatype3.html) rather than strict types. Any column can hold any type at runtime. Scythe maps declared types to neutral types based on the declared column type name.

```sql
CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    score REAL,
    data BLOB
);
```

| Affinity | Declared Types | Neutral Type |
|----------|---------------|-------------|
| INTEGER | `INTEGER`, `INT`, `SMALLINT`, `BIGINT`, `TINYINT`, `MEDIUMINT` | `int32` (or `int64` for BIGINT) |
| REAL | `REAL`, `FLOAT`, `DOUBLE`, `DOUBLE PRECISION` | `float32` / `float64` |
| TEXT | `TEXT`, `VARCHAR`, `CHAR`, `CLOB` | `string` |
| BLOB | `BLOB` | `bytes` |
| NUMERIC | `NUMERIC`, `DECIMAL`, `BOOLEAN`, `DATE`, `DATETIME` | varies |

## AUTOINCREMENT handling

```sql
CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL
);
```

`INTEGER PRIMARY KEY` is the SQLite auto-increment rowid. The `AUTOINCREMENT` keyword adds monotonicity. These columns are treated as NOT NULL `int32`.

## Limitations

SQLite does not support:

- **Enums** -- no `CREATE TYPE ... AS ENUM`. Use `TEXT` with `CHECK` constraints instead.
- **Arrays** -- no array types. Use JSON arrays or separate tables.
- **Schemas** -- no `schema.table` syntax. Single namespace per database.
- **Composite types** -- no `CREATE TYPE ... AS (...)`.
- **Range types** -- not available.
- **Network types** -- no `INET`, `CIDR`. Use `TEXT`.
- **RETURNING** -- only available in SQLite 3.35+ (2021).

## Type mapping table

| SQLite Type | Neutral Type |
|------------|-------------|
| `INTEGER` / `INT` | `int32` |
| `BIGINT` | `int64` |
| `SMALLINT` / `TINYINT` | `int16` |
| `MEDIUMINT` | `int32` |
| `REAL` / `FLOAT` | `float32` |
| `DOUBLE` / `DOUBLE PRECISION` | `float64` |
| `TEXT` / `VARCHAR` / `CHAR` / `CLOB` | `string` |
| `BLOB` | `bytes` |
| `BOOLEAN` / `BOOL` | `bool` |
| `NUMERIC` / `DECIMAL` | `decimal` |
| `DATE` | `date` |
| `DATETIME` | `datetime` |
| `JSON` | `json` |

## Placeholder syntax

SQLite uses `$N` positional placeholders, same as PostgreSQL:

```sql
SELECT id, name FROM users WHERE id = $1;
```
