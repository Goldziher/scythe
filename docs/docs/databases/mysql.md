# MySQL

Scythe supports MySQL/MariaDB with dialect-specific type handling.

## Differences from PostgreSQL

| Feature | PostgreSQL | MySQL |
|---------|-----------|-------|
| Auto-increment | `SERIAL` (type alias) | `AUTO_INCREMENT` (column modifier) |
| Enums | `CREATE TYPE ... AS ENUM` | `ENUM(...)` inline on column |
| Arrays | `TEXT[]` native | Not supported |
| Placeholders | `$1`, `$2` | `?`, `?` |
| JSONB | Native binary JSON | JSON only (no JSONB) |
| Schemas | `schema.table` | Database-scoped |
| RETURNING | Supported | Not supported (use `LAST_INSERT_ID()`) |
| Range types | Native | Not supported |

## AUTO_INCREMENT handling

```sql
CREATE TABLE users (
    id INT AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(255) NOT NULL
);
```

`AUTO_INCREMENT` columns are treated as NOT NULL, equivalent to PostgreSQL's `SERIAL`.

## Inline ENUM

MySQL enums are declared inline on the column:

```sql
CREATE TABLE users (
    id INT AUTO_INCREMENT PRIMARY KEY,
    status ENUM('active', 'inactive', 'banned') NOT NULL
);
```

These map to `string` in the neutral type system. There is no separate `enum::` resolution for inline MySQL enums.

## Placeholder handling

MySQL uses `?` positional placeholders. Scythe translates `$N` in your SQL to `?` for MySQL backends:

```sql
-- Written as:
SELECT id, name FROM users WHERE id = $1;

-- Translated to:
SELECT id, name FROM users WHERE id = ?;
```

## Type mapping table

| MySQL Type | Neutral Type |
|-----------|-------------|
| `INT` / `INTEGER` | `int32` |
| `BIGINT` | `int64` |
| `SMALLINT` | `int16` |
| `TINYINT` | `int16` |
| `MEDIUMINT` | `int32` |
| `FLOAT` | `float32` |
| `DOUBLE` | `float64` |
| `DECIMAL` / `NUMERIC` | `decimal` |
| `VARCHAR` / `CHAR` / `TEXT` | `string` |
| `TINYTEXT` / `MEDIUMTEXT` / `LONGTEXT` | `string` |
| `BOOLEAN` / `BOOL` | `bool` |
| `BIT` | `bool` |
| `BLOB` / `TINYBLOB` / `MEDIUMBLOB` / `LONGBLOB` | `bytes` |
| `BINARY` / `VARBINARY` | `bytes` |
| `DATE` | `date` |
| `TIME` | `time` |
| `DATETIME` | `datetime` |
| `TIMESTAMP` | `datetime` |
| `YEAR` | `int16` |
| `JSON` | `json` |
| `ENUM(...)` | `string` |
| `SET(...)` | `string` |
