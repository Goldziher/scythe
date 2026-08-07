//! Live-database conformance suite for scythe's nullability inference.
//!
//! This crate compares three facts per (fixture, engine, column):
//!
//! - **A** -- the analyzer's [`scythe_core::analyzer::AnalyzedColumn::nullable`]
//! - **G** -- whether the generated code actually *renders* the column as
//!   optional, parsed from `ResolvedColumn::full_type` against the backend
//!   manifest (see [`ColumnFacts::from_analyzed_and_generated`]) --
//!   deliberately not `ResolvedColumn::nullable`, which
//!   [`scythe_codegen::resolve::resolve_columns`] copies verbatim from `A`
//!   and so can never disagree with it
//! - **E** -- the engine's observed per-row nullness, from a real query run
//!   against a live database
//!
//! against four assertions (see [`verdict`] for the full definitions):
//! fidelity (A1, `A == G`), soundness (A2, an observed NULL implies the
//! generated code renders the column optional -- never suppressible),
//! anti-vacuity (A3, a nullable column must be demonstrated NULL by some
//! run, or the whole suite degenerates to marking everything nullable), and
//! join-group coherence (A4, sibling columns widened by the same outer join
//! go NULL together).
//!
//! No default feature links a database driver: [`fixture`], [`expectation`],
//! [`verdict`], [`divergence`], and [`query_shape`] are pure and DB-free, so
//! `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`
//! exercise them without a container. Per-engine drivers live behind the
//! `pg`, `mysql`, `mariadb`, `sqlite`, `mssql`, and `oracle` features (plus
//! the `live-tests` gate for actually dialing a database), implementing the
//! [`executor::Executor`] trait -- no implementation ships in this commit.

pub mod divergence;
pub mod executor;
pub mod expectation;
pub mod fixture;
pub mod query_shape;
pub mod verdict;

pub use divergence::{DivergenceEntry, DivergenceError, DivergenceKind};
pub use executor::{Executor, MissingColumn, ObservedRow};
pub use expectation::ExpectationError;
pub use fixture::{Engine, FixtureError, LiveFixture};
pub use verdict::{ColumnFacts, ColumnFactsError, Failure, FixtureEngineFacts, Verdict};
