# Neutral Types

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
| `array<T>` | `Vec<T>` | `list[T]` | `T[]` | `[]T` | `java.util.List<T>` | `List<T>` | `List<T>` | `list(T)` | `Array<T>` |
| `nullable` | `Option<T>` | `T \| None` | `T \| null` | `*T` | `@Nullable T` | `T?` | `T?` | `T \| nil` | `T` |
| `range<T>` | `PgRange<T>` | `tuple[T, T]` | `string` | `string` | `String` | `String` | `string` | `string()` | `String` |
| `json_typed<T>` | `sqlx::types::Json<T>` | `T` | `T` | `T` | `T` | `T` | `T` | `T` | `T` |

## Special types

| Neutral Pattern | Description | Example |
|----------------|-------------|---------|
| `enum::name` | User-defined PostgreSQL enum | `enum::user_status` becomes `UserStatus` |
| `composite::name` | User-defined composite type | `composite::address` becomes `Address` |

Enum and composite names are converted to PascalCase for all backends.

## SQL to neutral mapping

| SQL Type(s) | Neutral Type |
|------------|-------------|
| `INTEGER`, `INT`, `INT4`, `SERIAL` | `int32` |
| `SMALLINT`, `INT2`, `SMALLSERIAL` | `int16` |
| `BIGINT`, `INT8`, `BIGSERIAL` | `int64` |
| `REAL`, `FLOAT4`, `FLOAT` | `float32` |
| `DOUBLE PRECISION`, `FLOAT8` | `float64` |
| `NUMERIC`, `DECIMAL` | `decimal` |
| `TEXT`, `VARCHAR`, `CHAR` | `string` |
| `BOOLEAN`, `BOOL` | `bool` |
| `BYTEA`, `BLOB`, `BINARY` | `bytes` |
| `UUID` | `uuid` |
| `DATE` | `date` |
| `TIME` | `time` |
| `TIMETZ` | `time_tz` |
| `TIMESTAMP` | `datetime` |
| `TIMESTAMPTZ` | `datetime_tz` |
| `INTERVAL` | `interval` |
| `JSON`, `JSONB` | `json` |
| `INET`, `CIDR`, `MACADDR` | `inet` |
| `INTEGER[]` | `array<int32>` |
| `INT4RANGE` | `range<int32>` |
| `TSTZRANGE` | `range<datetime_tz>` |
