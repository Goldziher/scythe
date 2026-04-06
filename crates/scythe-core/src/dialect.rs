/// Supported SQL dialects for parsing and type resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SqlDialect {
    #[default]
    PostgreSQL,
    MySQL,
    SQLite,
}

impl SqlDialect {
    /// Convert to a boxed sqlparser dialect for use with the parser.
    pub fn to_sqlparser_dialect(&self) -> Box<dyn sqlparser::dialect::Dialect> {
        match self {
            SqlDialect::PostgreSQL => Box::new(sqlparser::dialect::PostgreSqlDialect {}),
            SqlDialect::MySQL => Box::new(sqlparser::dialect::MySqlDialect {}),
            SqlDialect::SQLite => Box::new(sqlparser::dialect::SQLiteDialect {}),
        }
    }

    /// Parse a dialect name from a string (case-insensitive).
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "postgresql" | "postgres" | "pg" => Some(Self::PostgreSQL),
            "mysql" | "mariadb" => Some(Self::MySQL),
            "sqlite" | "sqlite3" => Some(Self::SQLite),
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
    fn test_from_str_unknown() {
        assert_eq!(SqlDialect::from_str("oracle"), None);
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
    }
}
