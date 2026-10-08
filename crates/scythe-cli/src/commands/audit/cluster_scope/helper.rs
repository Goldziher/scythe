//! Locating and running the pinned PostgreSQL parser helper binaries.
//!
//! The gate never links libpg_query: the two grammars carry identical C
//! symbols, so they ship as two independent executables built at release time
//! ([`build_pg_parser_helpers.sh`]). This module resolves those executables and
//! enforces the transport contract from `PARSER_PROTOCOL.md`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::response::{ParseMode, ValidatedAst};

/// A pinned PostgreSQL grammar the gate can be run against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PgVersion {
    Pg15,
    #[default]
    Pg18,
}

impl PgVersion {
    /// `(pg_major, pg_version, parser_release)` as reported by the helper.
    pub fn expected(self) -> (u64, &'static str, &'static str) {
        match self {
            Self::Pg15 => (15, "15.1", "15-4.2.4"),
            Self::Pg18 => (18, "18.6", "18.1.0"),
        }
    }

    pub fn major(self) -> u64 {
        self.expected().0
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "15" | "pg15" | "postgres15" => Some(Self::Pg15),
            "18" | "pg18" | "postgres18" => Some(Self::Pg18),
            _ => None,
        }
    }
}

/// The two helper executables the gate needs, resolved once per run.
pub struct HelperSet {
    dir: PathBuf,
}

impl HelperSet {
    /// Resolve the helper directory.
    ///
    /// Order: `SCYTHE_PG_PARSER_DIR`, then the directory holding the running
    /// `scythe` executable (how release archives ship them). Missing helpers
    /// are an error, never a silent skip.
    pub fn discover() -> Result<Self, String> {
        if let Some(dir) = std::env::var_os("SCYTHE_PG_PARSER_DIR") {
            let dir = PathBuf::from(dir);
            if dir.as_os_str().is_empty() {
                return Err("SCYTHE_PG_PARSER_DIR is set but empty".to_string());
            }
            return Ok(Self { dir });
        }
        if let Ok(exe) = std::env::current_exe()
            && let Some(parent) = exe.parent()
        {
            return Ok(Self {
                dir: parent.to_path_buf(),
            });
        }
        Err("cannot locate the PostgreSQL parser helpers: set SCYTHE_PG_PARSER_DIR".to_string())
    }

    fn binary(&self, version: PgVersion) -> PathBuf {
        self.dir.join(format!("scythe-pg{}-parser", version.major()))
    }

    /// Run the helper for `version`/`mode` over `sql` and validate the reply.
    pub fn parse(&self, version: PgVersion, mode: ParseMode, sql: &str) -> Result<ValidatedAst, String> {
        let binary = self.binary(version);
        if !is_executable(&binary) {
            return Err(format!(
                "PostgreSQL {} parser helper not found or not executable at '{}'; \
                 build it with build_pg_parser_helpers.sh or set SCYTHE_PG_PARSER_DIR",
                version.major(),
                binary.display()
            ));
        }

        let mut command = Command::new(&binary);
        if mode == ParseMode::Plpgsql {
            command.arg("--plpgsql");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("failed to run parser helper '{}': {error}", binary.display()))?;
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(sql.as_bytes())
            .map_err(|error| format!("failed to send SQL to parser helper: {error}"))?;

        let output = child
            .wait_with_output()
            .map_err(|error| format!("failed to read parser helper output: {error}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "parser helper (PostgreSQL {}) failed: {}",
                version.major(),
                stderr.trim()
            ));
        }
        ValidatedAst::parse(&output.stdout, version, mode)
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && std::fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pg_version_reports_pinned_grammar() {
        assert_eq!(PgVersion::Pg15.expected(), (15, "15.1", "15-4.2.4"));
        assert_eq!(PgVersion::Pg18.expected(), (18, "18.6", "18.1.0"));
    }

    #[test]
    fn pg_version_parses_supported_spellings_only() {
        assert_eq!(PgVersion::parse("15"), Some(PgVersion::Pg15));
        assert_eq!(PgVersion::parse("pg18"), Some(PgVersion::Pg18));
        assert_eq!(PgVersion::parse("16"), None);
        assert_eq!(PgVersion::parse("klingon"), None);
    }

    #[test]
    fn missing_helper_is_an_error_not_a_skip() {
        // ~keep A helper directory with no binaries must fail, not report success.
        let dir = std::env::temp_dir().join(format!("scythe-pg-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let helpers = HelperSet { dir };
        assert!(helpers.parse(PgVersion::Pg18, ParseMode::Sql, "SELECT 1").is_err());
    }
}
