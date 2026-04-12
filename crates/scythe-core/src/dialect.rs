/// Supported SQL dialects for parsing and type resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SqlDialect {
    #[default]
    PostgreSQL,
    MySQL,
    SQLite,
    MsSql,
    Oracle,
    Snowflake,
}

impl SqlDialect {
    /// Convert to a boxed sqlparser dialect for use with the parser.
    pub fn to_sqlparser_dialect(&self) -> Box<dyn sqlparser::dialect::Dialect> {
        match self {
            SqlDialect::PostgreSQL => Box::new(sqlparser::dialect::PostgreSqlDialect {}),
            SqlDialect::MySQL => Box::new(sqlparser::dialect::MySqlDialect {}),
            SqlDialect::SQLite => Box::new(sqlparser::dialect::SQLiteDialect {}),
            SqlDialect::MsSql => Box::new(sqlparser::dialect::MsSqlDialect {}),
            SqlDialect::Oracle => Box::new(sqlparser::dialect::OracleDialect {}),
            SqlDialect::Snowflake => Box::new(sqlparser::dialect::SnowflakeDialect {}),
        }
    }

    /// Parse a dialect name from a string (case-insensitive).
    /// Returns `Option<Self>` instead of `Result` since unknown dialects are not errors.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "postgresql" | "postgres" | "pg" | "cockroachdb" | "crdb" => Some(Self::PostgreSQL),
            "mysql" | "mariadb" => Some(Self::MySQL),
            "sqlite" | "sqlite3" => Some(Self::SQLite),
            "duckdb" | "redshift" => Some(Self::PostgreSQL),
            "mssql" | "sqlserver" | "tsql" => Some(Self::MsSql),
            "oracle" => Some(Self::Oracle),
            "snowflake" => Some(Self::Snowflake),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_str_postgresql() {
        assert_eq!(
            SqlDialect::from_str("postgresql"),
            Some(SqlDialect::PostgreSQL)
        );
        assert_eq!(
            SqlDialect::from_str("postgres"),
            Some(SqlDialect::PostgreSQL)
        );
        assert_eq!(SqlDialect::from_str("pg"), Some(SqlDialect::PostgreSQL));
        assert_eq!(
            SqlDialect::from_str("PostgreSQL"),
            Some(SqlDialect::PostgreSQL)
        );
    }

    #[test]
    fn test_from_str_mysql() {
        assert_eq!(SqlDialect::from_str("mysql"), Some(SqlDialect::MySQL));
        assert_eq!(SqlDialect::from_str("mariadb"), Some(SqlDialect::MySQL));
        assert_eq!(SqlDialect::from_str("MySQL"), Some(SqlDialect::MySQL));
    }

    #[test]
    fn test_from_str_sqlite() {
        assert_eq!(SqlDialect::from_str("sqlite"), Some(SqlDialect::SQLite));
        assert_eq!(SqlDialect::from_str("sqlite3"), Some(SqlDialect::SQLite));
    }

    #[test]
    fn test_from_str_cockroachdb() {
        assert_eq!(
            SqlDialect::from_str("cockroachdb"),
            Some(SqlDialect::PostgreSQL)
        );
        assert_eq!(SqlDialect::from_str("crdb"), Some(SqlDialect::PostgreSQL));
        assert_eq!(
            SqlDialect::from_str("CockroachDB"),
            Some(SqlDialect::PostgreSQL)
        );
    }

    #[test]
    fn test_from_str_duckdb() {
        assert_eq!(SqlDialect::from_str("duckdb"), Some(SqlDialect::PostgreSQL));
        assert_eq!(SqlDialect::from_str("DuckDB"), Some(SqlDialect::PostgreSQL));
    }

    #[test]
    fn test_from_str_mssql() {
        assert_eq!(SqlDialect::from_str("mssql"), Some(SqlDialect::MsSql));
        assert_eq!(SqlDialect::from_str("sqlserver"), Some(SqlDialect::MsSql));
        assert_eq!(SqlDialect::from_str("tsql"), Some(SqlDialect::MsSql));
        assert_eq!(SqlDialect::from_str("MSSQL"), Some(SqlDialect::MsSql));
    }

    #[test]
    fn test_from_str_oracle() {
        assert_eq!(SqlDialect::from_str("oracle"), Some(SqlDialect::Oracle));
        assert_eq!(SqlDialect::from_str("Oracle"), Some(SqlDialect::Oracle));
    }

    #[test]
    fn test_from_str_snowflake() {
        assert_eq!(
            SqlDialect::from_str("snowflake"),
            Some(SqlDialect::Snowflake)
        );
        assert_eq!(
            SqlDialect::from_str("Snowflake"),
            Some(SqlDialect::Snowflake)
        );
    }

    #[test]
    fn test_from_str_redshift() {
        assert_eq!(
            SqlDialect::from_str("redshift"),
            Some(SqlDialect::PostgreSQL)
        );
        assert_eq!(
            SqlDialect::from_str("Redshift"),
            Some(SqlDialect::PostgreSQL)
        );
    }

    #[test]
    fn test_from_str_mariadb() {
        assert_eq!(SqlDialect::from_str("mariadb"), Some(SqlDialect::MySQL));
        assert_eq!(SqlDialect::from_str("MariaDB"), Some(SqlDialect::MySQL));
    }

    #[test]
    fn test_from_str_unknown() {
        assert_eq!(SqlDialect::from_str("nosuchdb"), None);
        assert_eq!(SqlDialect::from_str(""), None);
    }

    #[test]
    fn test_default_is_postgresql() {
        assert_eq!(SqlDialect::default(), SqlDialect::PostgreSQL);
    }

    #[test]
    fn test_to_sqlparser_dialect() {
        // Just verify they don't panic
        let _ = SqlDialect::PostgreSQL.to_sqlparser_dialect();
        let _ = SqlDialect::MySQL.to_sqlparser_dialect();
        let _ = SqlDialect::SQLite.to_sqlparser_dialect();
        let _ = SqlDialect::MsSql.to_sqlparser_dialect();
        let _ = SqlDialect::Oracle.to_sqlparser_dialect();
        let _ = SqlDialect::Snowflake.to_sqlparser_dialect();
    }
}
