// @ts-check
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import starlightLlmsTxt from "starlight-llms-txt";

export default defineConfig({
  site: "https://goldziher.github.io",
  base: "/scythe",
  integrations: [
    starlight({
      title: "Scythe",
      description: "Polyglot SQL-to-code generator with built-in linting and formatting",
      logo: {
        src: "./src/assets/logo.svg",
        alt: "Scythe",
      },
      favicon: "/favicon.svg",
      customCss: ["./src/styles/custom.css"],
      social: [{ icon: "github", label: "GitHub", href: "https://github.com/Goldziher/scythe" }],
      editLink: {
        baseUrl: "https://github.com/Goldziher/scythe/edit/main/website/",
      },
      plugins: [
        starlightLlmsTxt({
          description:
            "Scythe compiles annotated SQL into type-safe database access code for 10 languages " +
            "(Rust, Python, TypeScript, Go, Java, Kotlin, C#, Elixir, Ruby, PHP — plus plain " +
            "JavaScript, emitted by the TypeScript backends' JSDoc mode) across 10 databases " +
            "(PostgreSQL, MySQL, MariaDB, SQLite, DuckDB, CockroachDB, MSSQL, Oracle, Redshift, " +
            "Snowflake) via 56 backends, with built-in SQL linting, security auditing and " +
            "formatting.",
          // Facts a model cannot infer from the page bodies but will get wrong
          // without. The sqlc point is the most load-bearing: scythe's
          // annotation syntax is deliberately different, so a model that pattern-
          // matches on sqlc will emit `-- name: X :one` and produce a config that
          // does not parse.
          details: [
            "Important notes:",
            "",
            "- Queries are annotated with `-- @name X` and `-- @returns :many`. This is **not** sqlc's `-- name: X :one` syntax; scythe is not syntax-compatible with sqlc.",
            "- Generated code is written to disk and committed to the repository. Generation needs no database connection and no network access.",
            "- Every generated file's first line is a `scythe:provenance` header recording the backend, engine, and fingerprints of the schema and queries it came from. `scythe check` uses it to detect drift.",
            "- `scythe check` exits 2 on error-severity findings and 1 on operational failure. This differs from `scythe lint`, which exits 1.",
          ].join("\n"),
          optionalLinks: [
            { label: "Source repository", url: "https://github.com/Goldziher/scythe" },
            {
              label: "Changelog",
              url: "https://github.com/Goldziher/scythe/blob/main/CHANGELOG.md",
              description: "Release history — excluded from llms-small.txt to keep it focused on usage.",
            },
            { label: "crates.io (scythe-cli)", url: "https://crates.io/crates/scythe-cli" },
            { label: "PyPI (scythe-sql)", url: "https://pypi.org/project/scythe-sql/" },
            { label: "npm (scythe-cli)", url: "https://www.npmjs.com/package/scythe-cli" },
          ],
          // Addressable subsets, so a model after one backend can fetch
          // /_llms-txt/<slug>.txt instead of the ~500 KB llms-full.txt.
          customSets: [
            {
              label: "Backends",
              paths: ["backends/**"],
              description: "Per-language backend reference: options, type mappings, and emitted code shape.",
            },
            {
              label: "Databases",
              paths: ["databases/**"],
              description: "Per-engine notes: supported backends, dialect quirks, and manifest conventions.",
            },
            {
              label: "Guide",
              paths: ["guide/**"],
              description: "Task-oriented guides: configuration, annotations, type inference, linting, audit, inspect.",
            },
          ],
          // One pattern per page rather than `getting-started/**`: pages within a
          // single pattern still sort alphabetically by id, which wedged
          // migration-from-sqlc between installation and quickstart.
          promote: [
            "index*",
            "getting-started/installation",
            "getting-started/quickstart",
            "getting-started/migration-from-sqlc",
            "philosophy",
            "architecture",
          ],
          // `exclude` only applies to llms-small.txt, so the changelog is also
          // demoted to push it to the tail of llms-full.txt -- context-window
          // truncation should drop release history before it drops reference
          // material.
          exclude: ["reference/changelog"],
          demote: ["reference/changelog", "comparisons/alternatives"],
          // `collapseCodeBlocks` is deliberately NOT set. It is the plugin's
          // back-compat escape hatch, and it collapses whitespace *inside* code
          // fences -- which for a code generator's documentation destroys the
          // single most valuable thing in it. With it on, llms-small.txt rendered
          // every Python sample as one unparseable line, and bought a 15% size
          // reduction for it.
          minify: {},
          // Starlight renders a "Section titled ..." anchor link after every
          // heading. In the built corpus that is 486 fragments / ~24 KB / 4.8%,
          // and it is pure navigation chrome with no meaning to a model.
          customSelectors: { all: [".sl-anchor-link"] },
          // The default "\n\n" is ambiguous here: this project's docs are full of
          // shell and Python comment lines that begin with `#`, which read
          // exactly like the `# Page Title` headings separating pages.
          pageSeparator: "\n\n---\n\n",
        }),
      ],
      sidebar: [
        { label: "Philosophy", slug: "philosophy" },
        { label: "Architecture", slug: "architecture" },
        {
          label: "Getting Started",
          items: [
            { label: "Installation", slug: "getting-started/installation" },
            { label: "Quickstart", slug: "getting-started/quickstart" },
            { label: "Migration from sqlc", slug: "getting-started/migration-from-sqlc" },
          ],
        },
        {
          label: "Comparisons",
          items: [{ label: "Alternatives", slug: "comparisons/alternatives" }],
        },
        {
          label: "Guide",
          items: [
            { label: "Configuration", slug: "guide/configuration" },
            { label: "Annotations", slug: "guide/annotations" },
            { label: "Type Inference", slug: "guide/type-inference" },
            { label: "Custom Types", slug: "guide/custom-types" },
            { label: "Linting", slug: "guide/linting" },
            { label: "Audit", slug: "guide/audit" },
            { label: "Inspect", slug: "guide/inspect" },
            { label: "Formatting", slug: "guide/formatting" },
            { label: "Pre-commit Hooks", slug: "guide/pre-commit-hooks" },
            { label: "CLI Reference", slug: "guide/cli-reference" },
            { label: "Migration from sqlfluff", slug: "guide/migration-from-sqlfluff" },
          ],
        },
        {
          label: "Backends",
          items: [
            { label: "Overview", slug: "backends/overview" },
            { label: "Rust (sqlx)", slug: "backends/rust-sqlx" },
            { label: "Rust (tokio-postgres)", slug: "backends/rust-tokio-postgres" },
            { label: "Python", slug: "backends/python" },
            { label: "TypeScript", slug: "backends/typescript" },
            { label: "Go", slug: "backends/go" },
            { label: "Java", slug: "backends/java" },
            { label: "Kotlin", slug: "backends/kotlin" },
            { label: "C#", slug: "backends/csharp" },
            { label: "Elixir", slug: "backends/elixir" },
            { label: "PHP", slug: "backends/php" },
            { label: "Ruby", slug: "backends/ruby" },
          ],
        },
        {
          label: "Databases",
          items: [
            { label: "PostgreSQL", slug: "databases/postgresql" },
            { label: "MySQL", slug: "databases/mysql" },
            { label: "SQLite", slug: "databases/sqlite" },
            { label: "DuckDB", slug: "databases/duckdb" },
            { label: "CockroachDB", slug: "databases/cockroachdb" },
            { label: "MSSQL", slug: "databases/mssql" },
            { label: "Oracle", slug: "databases/oracle" },
            { label: "MariaDB", slug: "databases/mariadb" },
            { label: "Redshift", slug: "databases/redshift" },
            { label: "Snowflake", slug: "databases/snowflake" },
          ],
        },
        {
          label: "Examples",
          items: [
            { label: "Simple CRUD", slug: "examples/simple-crud" },
            { label: "Pagila", slug: "examples/pagila" },
            { label: "BaseMind", slug: "examples/basemind" },
          ],
        },
        {
          label: "Reference",
          items: [
            { label: "Neutral Types", slug: "reference/neutral-types" },
            { label: "Lint Rules", slug: "reference/lint-rules" },
            { label: "Changelog", slug: "reference/changelog" },
          ],
        },
      ],
    }),
  ],
});
