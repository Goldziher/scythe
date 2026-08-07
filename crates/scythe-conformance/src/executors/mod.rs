//! Per-engine live drivers, each behind its own Cargo feature so
//! `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`
//! never link a database driver by default. See [`crate::runner`] for how
//! these are dispatched -- [`crate::executor::Executor`] is not
//! dyn-compatible, so callers select a concrete type per engine rather than
//! holding these behind a trait object.

#[cfg(feature = "pg")]
pub mod postgres;

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(any(feature = "mysql", feature = "mariadb"))]
pub mod mysql;
