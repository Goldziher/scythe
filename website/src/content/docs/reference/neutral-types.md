---
title: Neutral Types
description: Complete type mapping table across all supported languages -- scalars, containers, and special types.
---

Neutral types are scythe's intermediate representation between SQL types and language types. The analyzer converts SQL types to neutral types; backend manifests map neutral types to language-specific types.

## Scalar types

| Neutral | Rust (sqlx) | Rust (tokio-pg) | Python | TypeScript | Go | Java | Kotlin | C# | Elixir | Ruby |
|---------|------------|-----------------|--------|-----------|-----|------|--------|-----|--------|------|
| `bool` | `bool` | `bool` | `bool` | `boolean` | `bool` | `boolean` | `Boolean` | `bool` | `boolean()` | `Boolean` |
| `int16` | `i16` | `i16` | `int` | `number` | `int16` | `short` | `Short` | `short` | `integer()` | `Integer` |
| `int32` | `i32` | `i32` | `int` | `number` | `int32` | `int` | `Int` | `int` | `integer()` | `Integer` |
| `int64` | `i64` | `i64` | `int` | `number` | `int64` | `long` | `Long` | `long` | `integer()` | `Integer` |
| `float32` | `f32` | `f32` | `float` | `number` | `float32` | `float` | `Float` | `float` | `float()` | `Float` |
| `float64` | `f64` | `f64` | `float` | `number` | `float64` | `double` | `Double` | `double` | `float()` | `Float` |
| `string` | `String` | `String` | `str` | `string` | `string` | `String` | `String` | `string` | `String.t()` | `String` |
| `bytes` | `Vec<u8>` | `Vec<u8>` | `bytes` | `Buffer` | `[]byte` | `byte[]` | `ByteArray` | `byte[]` | `binary()` | `String` |
| `uuid` | `uuid::Uuid` | `uuid::Uuid` | `uuid.UUID` | `string` | `uuid.UUID` | `java.util.UUID` | `java.util.UUID` | `Guid` | `String.t()` | `String` |
| `decimal` | `rust_decimal::Decimal` | `rust_decimal::Decimal` | `decimal.Decimal` | `string` | `decimal.Decimal` | `java.math.BigDecimal` | `java.math.BigDecimal` | `decimal` | `Decimal.t()` | `BigDecimal` |
| `date` | `chrono::NaiveDate` | `chrono::NaiveDate` | `datetime.date` | `string` | `time.Time` | `java.time.LocalDate` | `java.time.LocalDate` | `DateOnly` | `Date.t()` | `Date` |
| `time` | `chrono::NaiveTime` | `chrono::NaiveTime` | `datetime.time` | `string` | `time.Time` | `java.time.LocalTime` | `java.time.LocalTime` | `TimeOnly` | `Time.t()` | `Time` |
| `time_tz` | `sqlx::postgres::types::PgTimeTz` | `chrono::NaiveTime` | `datetime.time` | `string` | `time.Time` | `java.time.OffsetTime` | `java.time.OffsetTime` | `TimeOnly` | `Time.t()` | `Time` |
| `datetime` | `chrono::NaiveDateTime` | `chrono::NaiveDateTime` | `datetime.datetime` | `Date` | `time.Time` | `java.time.LocalDateTime` | `java.time.LocalDateTime` | `DateTime` | `NaiveDateTime.t()` | `Time` |
| `datetime_tz` | `chrono::DateTime<chrono::Utc>` | `chrono::DateTime<chrono::Utc>` | `datetime.datetime` | `Date` | `time.Time` | `java.time.OffsetDateTime` | `java.time.OffsetDateTime` | `DateTimeOffset` | `DateTime.t()` | `Time` |
| `interval` | `sqlx::postgres::types::PgInterval` | `String` | `datetime.timedelta` | `string` | `time.Duration` | `String` | `String` | `TimeSpan` | `String.t()` | `String` |
| `json` | `serde_json::Value` | `serde_json::Value` | `dict[str, Any]` | `Record<string, unknown>` | `json.RawMessage` | `String` | `String` | `string` | `map()` | `Hash` |
| `inet` | `ipnetwork::IpNetwork` | `std::net::IpAddr` | `str` | `string` | `netip.Addr` | `String` | `String` | `System.Net.IPAddress` | `String.t()` | `String` |

## Container types

| Neutral Pattern | Rust (sqlx) | Python | TypeScript | Go | Java | Kotlin | C# | Elixir | Ruby |
|----------------|------------|--------|-----------|-----|------|--------|-----|--------|------|
| `array<T>` | `Vec<T>` | `list[T]` | `T[]` | `[]T` | `String` | `String` | `List<T>` | `list(T)` | `Array<T>` |
| `nullable` | `Option<T>` | `T \| None` | `T \| null` | `*T` | `@Nullable T` | `T?` | `T?` | `T \| nil` | `T` |
| `range<T>` | `sqlx::postgres::types::PgRange<T>` | `tuple[T, T]` | `string` | `string` | `String` | `String` | `string` | `string()` | `String` |
| `json_typed<T>` | `sqlx::types::Json<T>` | `T` | `T` | `T` | `T` | `T` | `T` | `T` | `T` |
| `json_nested<T>` | `sqlx::types::Json<T>` | `T` | -- | `T` | -- | -- | -- | -- | -- |

`array<T>` maps to `String` on Java and Kotlin, not to a list. Neither backend has
an array reader -- every non-scalar column is read through the untyped accessor
(`ResultSet.getObject`, `Row.get(col, Object.class)`), whose static type is
`Object`/`Any`, so a list-typed declaration does not compile. The column arrives
as the driver's text form, the same way `range<T>` and `json` already do. This is
a limitation, not a design choice; typed array readers are tracked separately.

`json_typed<T>` is what your own `@json` annotation produces: `T` is a type you
declared and scythe knows nothing about its shape.

`json_nested<T>` is what `json_agg(alias.*)` and `row_to_json(alias.*)` produce on
PostgreSQL: `T` is a struct scythe synthesizes from the aggregated relation, field
by field.

Four backends decode it -- `rust-sqlx` (`sqlx::types::Json<T>`),
`rust-tokio-postgres` (`postgres_types::Json<T>`), `go-pgx` (`T`) and
`python-psycopg3` (`T`). Every other backend degrades the column to plain `json`,
so its output is byte-identical to what it produced before this existed. Redshift
is excluded even on those four: it maps to the PostgreSQL dialect but has no
`json_agg`.

`json_agg` over an outer join yields `json_nested<array<nullable<T>>>`, because
PostgreSQL makes the whole-row variable NULL for a non-matching row and the
aggregate is then the JSON array `[null]`.

`json_typed<T>` is produced by the [`@json`](/scythe/databases/postgresql/#postgresql-specific-annotations)
annotation, which binds a JSON/JSONB column to a specific language type `T` instead of the generic `json` mapping.

## Special types

| Neutral Pattern | Description | Example |
|----------------|-------------|---------|
| `enum::name` | User-defined PostgreSQL enum | `enum::user_status` becomes `UserStatus` |
| `composite::name` | User-defined composite type | `composite::address` becomes `Address` |

Enum and composite names are converted to PascalCase for all backends.

## SQL to neutral mapping

This table has no separate dialect column; instead, dialect-specific exceptions are called out in
Notes. Where no exception is listed, the mapping is the same across all dialects that have the type.

| SQL Type(s) | Neutral Type | Notes |
|------------|-------------|-------|
| `INTEGER`, `INT`, `INT4`, `SERIAL` | `int32` | `INTEGER`/`INT` are `int64` on SQLite and Snowflake -- neither has a narrower storage class. `INT4`/`SERIAL` are PostgreSQL-only spellings and always stay `int32` |
| `SMALLINT`, `INT2`, `SMALLSERIAL` | `int16` | `SMALLINT` is `int64` on Snowflake -- every integer type is `NUMBER(38,0)` there. `INT2`/`SMALLSERIAL` are PostgreSQL-only spellings and always stay `int16` |
| `BIGINT`, `INT8`, `BIGSERIAL` | `int64` | |
| `HUGEINT`, `UHUGEINT` | `decimal` | DuckDB 128-bit integers; no neutral type is wide enough to hold them losslessly |
| `REAL`, `FLOAT4` | `float32` | `float64` on SQLite and Snowflake |
| `DOUBLE PRECISION`, `FLOAT8` | `float64` | |
| `FLOAT` (bare, no precision) | `float64` | `float32` on MySQL |
| `NUMERIC`, `DECIMAL` | `decimal` | |
| `NUMBER(p, s)`, `s > 0` | `decimal` | Oracle/Snowflake |
| `NUMBER(p)` / `NUMBER(p, 0)` | `int64` | Oracle/Snowflake; see the [Oracle](/scythe/databases/oracle/) page for the full split |
| `MONEY`, `SMALLMONEY` | `decimal` | |
| MySQL `UNSIGNED` family (e.g. `INT UNSIGNED`) | same-width signed neutral type | No dedicated unsigned neutral type; see [MySQL](/scythe/databases/mysql/) |
| `TEXT`, `VARCHAR`, `CHAR` | `string` | |
| `XML` | `string` | |
| `CLOB`, `NCLOB` | `string` | Oracle |
| `SET(...)` | `string` | MySQL/MariaDB |
| `ENUM(...)` | `enum::{table}_{column}` | MySQL/SQLite; `string` on other dialects |
| `GEOGRAPHY`, `GEOMETRY` | `string` | Snowflake, Redshift |
| `BOOLEAN`, `BOOL` | `bool` | |
| `BIT` / `BIT(1)` | `bool` | |
| `BIT(n)`, `n > 1` | `int64` | MySQL; `bytes` on other dialects with a `BIT(n)` type |
| `BYTEA`, `BLOB`, `BINARY` | `bytes` | |
| `BFILE` | `bytes` | Oracle |
| `UUID` | `uuid` | |
| `DATE` | `date` | |
| `TIME` | `time` | |
| `TIMETZ` | `time_tz` | |
| `TIMESTAMP` | `datetime` | |
| `TIMESTAMPTZ` | `datetime_tz` | |
| `TIMESTAMP_NTZ` | `datetime` | Snowflake |
| `TIMESTAMP_LTZ`, `TIMESTAMP_TZ` | `datetime_tz` | Snowflake |
| `YEAR` | `int16` | MySQL |
| `INTERVAL` | `interval` | |
| `JSON`, `JSONB` | `json` | |
| `VARIANT` | `json` | Snowflake |
| `SUPER` | `json` | Redshift |
| `INET`, `CIDR`, `MACADDR` | `inet` | |
| `INTEGER[]` | `array<int32>` | |
| `INT4RANGE` | `range<int32>` | |
| `TSRANGE` | `range<datetime>` | |
| `TSTZRANGE` | `range<datetime_tz>` | |
