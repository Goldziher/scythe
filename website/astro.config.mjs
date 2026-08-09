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
          promote: ["index*", "getting-started/**", "philosophy", "architecture"],
          minify: { collapseCodeBlocks: true },
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
