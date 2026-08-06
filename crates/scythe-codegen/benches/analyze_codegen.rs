//! Benchmarks for the analyzer + codegen path.
//!
//! Exercises a representative multi-table schema (enums, foreign keys, a
//! junction table) against a query set covering joins, aggregates, and
//! `GROUP BY` — the same shape of SQL used across `integration_tests/sql/pg`
//! — through parsing, type analysis, and code generation for two backends of
//! different weight (`sqlx`: template rendering only; `typescript-kysely`:
//! discriminated-union join grouping plus Zod schema emission).
//!
//! Run with `task bench` or `cargo bench -p scythe-codegen --bench analyze_codegen`.

use criterion::{Criterion, criterion_group, criterion_main};
use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;
use std::hint::black_box;

const SCHEMA: &str = "\
CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');

CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT,
    status user_status NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    user_id INT NOT NULL REFERENCES users (id),
    total NUMERIC(10, 2) NOT NULL,
    weight_kg DOUBLE PRECISION,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE tags (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE user_tags (
    user_id INT NOT NULL REFERENCES users (id),
    tag_id INT NOT NULL REFERENCES tags (id),
    PRIMARY KEY (user_id, tag_id)
);
";

/// Query set covering: plain select, filtered select, insert...returning,
/// update, delete, a LEFT JOIN (outer-join nullability widening), a
/// GROUP BY/HAVING aggregate, a two-way INNER JOIN, and a LIKE search —
/// mirrors `integration_tests/sql/pg/queries/{users,orders}.sql`.
const QUERIES: &[&str] = &[
    "-- @name GetUserById\n-- @returns :one\n\
     SELECT id, name, email, status, created_at FROM users WHERE id = $1;",
    "-- @name ListActiveUsers\n-- @returns :many\n\
     SELECT id, name, email FROM users WHERE status = $1;",
    "-- @name CreateUser\n-- @returns :one\n\
     INSERT INTO users (name, email, status) VALUES ($1, $2, $3) \
     RETURNING id, name, email, status, created_at;",
    "-- @name UpdateUserEmail\n-- @returns :exec\n\
     UPDATE users SET email = $1 WHERE id = $2;",
    "-- @name DeleteUser\n-- @returns :exec\n\
     DELETE FROM users WHERE id = $1;",
    "-- @name GetUserOrders\n-- @returns :many\n\
     SELECT u.id, u.name, o.total, o.notes \
     FROM users u LEFT JOIN orders o ON u.id = o.user_id \
     WHERE u.status = $1;",
    "-- @name CountUsersByStatus\n-- @returns :one\n\
     SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = $1;",
    "-- @name GetUserWithTags\n-- @returns :many\n\
     SELECT u.id, u.name, t.name AS tag_name \
     FROM users u \
     INNER JOIN user_tags ut ON u.id = ut.user_id \
     INNER JOIN tags t ON ut.tag_id = t.id \
     WHERE u.id = $1;",
    "-- @name SearchUsers\n-- @returns :many\n\
     SELECT id, name, email FROM users WHERE name LIKE $1;",
    "-- @name CreateOrder\n-- @returns :one\n\
     INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3) \
     RETURNING id, user_id, total, notes, created_at;",
    "-- @name GetOrdersByUser\n-- @returns :many\n\
     SELECT id, total, notes, created_at FROM orders WHERE user_id = $1 ORDER BY created_at DESC;",
    "-- @name GetOrderTotal\n-- @returns :one\n\
     SELECT SUM(total) AS total_sum FROM orders WHERE user_id = $1;",
];

fn build_catalog() -> Catalog {
    Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("valid schema")
}

/// Parse + analyze the full query set against the catalog. This is the path
/// that walks every expression node through `infer_expr_type`.
fn analyze_all(catalog: &Catalog) -> Vec<scythe_core::analyzer::AnalyzedQuery> {
    QUERIES
        .iter()
        .map(|sql| {
            let parsed = parse_query_with_dialect(sql, &SqlDialect::PostgreSQL).expect("valid query");
            analyze(catalog, &parsed).expect("analyzable query")
        })
        .collect()
}

fn bench_analyze(c: &mut Criterion) {
    let catalog = build_catalog();
    c.bench_function("analyze_query_set", |b| {
        b.iter(|| black_box(analyze_all(black_box(&catalog))));
    });
}

/// Analyzer only, with parsing hoisted out of the timed section. Isolates the
/// `infer_expr_type` recursion (and its per-node `TypeInfo` allocations) from
/// `sqlparser`'s tokenizing/AST-building cost, which otherwise dominates the
/// combined parse+analyze timing above.
fn bench_analyze_only(c: &mut Criterion) {
    let catalog = build_catalog();
    let parsed: Vec<_> = QUERIES
        .iter()
        .map(|sql| parse_query_with_dialect(sql, &SqlDialect::PostgreSQL).expect("valid query"))
        .collect();

    c.bench_function("analyze_only_query_set", |b| {
        b.iter(|| {
            for query in &parsed {
                black_box(analyze(black_box(&catalog), black_box(query)).expect("analyzable query"));
            }
        });
    });
}

fn bench_codegen_sqlx(c: &mut Criterion) {
    let catalog = build_catalog();
    let analyzed = analyze_all(&catalog);
    let backend = get_backend("sqlx", "postgresql").expect("sqlx backend");

    c.bench_function("codegen_sqlx", |b| {
        b.iter(|| {
            for query in &analyzed {
                let code = generate_with_backend(black_box(query), &*backend).expect("codegen succeeds");
                black_box(code);
            }
        });
    });
}

fn bench_codegen_typescript_kysely(c: &mut Criterion) {
    let catalog = build_catalog();
    let analyzed = analyze_all(&catalog);
    let backend = get_backend("typescript-kysely", "postgresql").expect("typescript-kysely backend");

    c.bench_function("codegen_typescript_kysely", |b| {
        b.iter(|| {
            for query in &analyzed {
                let code = generate_with_backend(black_box(query), &*backend).expect("codegen succeeds");
                black_box(code);
            }
        });
    });
}

fn bench_end_to_end(c: &mut Criterion) {
    let catalog = build_catalog();
    let sqlx_backend = get_backend("sqlx", "postgresql").expect("sqlx backend");
    let kysely_backend = get_backend("typescript-kysely", "postgresql").expect("typescript-kysely backend");

    c.bench_function("end_to_end_parse_analyze_generate", |b| {
        b.iter(|| {
            let analyzed = analyze_all(black_box(&catalog));
            for query in &analyzed {
                black_box(generate_with_backend(query, &*sqlx_backend).expect("codegen succeeds"));
                black_box(generate_with_backend(query, &*kysely_backend).expect("codegen succeeds"));
            }
        });
    });
}

criterion_group!(
    benches,
    bench_analyze,
    bench_analyze_only,
    bench_codegen_sqlx,
    bench_codegen_typescript_kysely,
    bench_end_to_end
);
criterion_main!(benches);
