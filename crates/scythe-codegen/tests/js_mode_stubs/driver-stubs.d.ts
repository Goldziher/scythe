// ~keep Hand-written ambient stubs for the driver packages the `javascript-*`
// (JSDoc emit mode, #81/#93) backends reference via `import("pkg").Type`
// JSDoc annotations: `pg`, `postgres`, `mysql2/promise`, `better-sqlite3`,
// `node:sqlite`.
//
// `tsc --checkJs --strict` (see `validate_javascript_tools` in
// `src/validation.rs`) needs these to resolve those `import("pkg")` type
// queries. The real `@types/pg` / `postgres` / `mysql2` / `better-sqlite3`
// packages would require `npm install` at test time -- network-dependent
// and not reproducible offline in CI -- so these are minimal, hand-written
// approximations instead, checked in alongside the test that uses them.
//
// These are NOT copies of the real `@types/*` packages. They mirror only
// the call shapes the generated code actually exercises, and were shaped
// deliberately to reproduce (not paper over) the exact untyped-return
// behavior that produced real `tsc` failures against the genuine npm
// packages during #81's development:
//   - `pg`'s `PoolClient.query` defaults its row-type generic to `any`
//     (matching @types/pg exactly), so `javascript-pg` needs no cast.
//   - `postgres`'s tagged-template `Sql` call defaults to a concrete `Row`
//     shape (an index signature, not `any`), so `javascript-postgres`'s
//     `:one`/`:many` paths fail to type-check without the
//     `/** @type {Array<T>} */ (...)` cast in
//     `typescript_postgres.rs::generate_query_fn_js`.
//   - `mysql2/promise`'s `execute(...)` returns an untyped `QueryResult`
//     union that is not directly destructurable, so `javascript-mysql2`
//     needs the tuple cast in `typescript_mysql2.rs::generate_query_fn_js`
//     and `generate_grouped_query_fn_js`.
//   - `better-sqlite3`'s `Statement.all()`/`.get()` return `unknown[]` /
//     `unknown`, so `javascript-better-sqlite3` needs the
//     `Array<Record<string, unknown>>` cast in
//     `typescript_better_sqlite3.rs::generate_grouped_query_fn_js`.
//   - `node:sqlite`'s `StatementSync.all()`/`.get()` return the real
//     `@types/node` shapes `Record<string, SQLOutputValue>[]` /
//     `Record<string, SQLOutputValue> | undefined` (copied from
//     `@types/node@26.1.2`'s `sqlite.d.ts`, not `unknown` like
//     better-sqlite3's stub above). This is the shape that makes the *TS*
//     backend need `as unknown as` for `:many`: `stmt.all() as Row[]` is a
//     genuine TS2352 ("neither type sufficiently overlaps"). The JSDoc
//     spelling of the same assertion is not -- `tsc --checkJs --strict`
//     accepts `/** @type {Row[]} */ (stmt.all())` against this exact
//     declaration -- so `javascript-node-sqlite` casts in one step
//     everywhere, and a second `unknown` hop here would be dead weight.
//
// If any of those casts is ever dropped, the corresponding
// `test_javascript_*_grouped_and_nullable_pass_real_tools` test in
// `tool_validation.rs` fails against these stubs exactly as it failed
// against the real packages when the defect was first found.

declare module "pg" {
  export interface PoolClient {
    query<T = any>(text: string, params?: unknown[]): Promise<{ rows: T[]; rowCount: number | null }>;
  }
}

declare module "postgres" {
  export interface Row {
    [column: string]: unknown;
  }
  export interface RowList<T> extends Array<T> {
    count: number;
  }
  export interface Sql {
    <T extends Row = Row>(strings: TemplateStringsArray, ...values: unknown[]): Promise<RowList<T>>;
    begin<T>(callback: (tx: Sql) => Promise<T>): Promise<T>;
  }
}

declare module "mysql2/promise" {
  export interface RowDataPacket {
    [column: string]: unknown;
  }
  export interface ResultSetHeader {
    affectedRows: number;
    insertId: number;
  }
  export type QueryResult = ResultSetHeader | RowDataPacket[] | RowDataPacket[][];
  export interface PoolConnection {
    execute(sql: string, values?: unknown[]): Promise<QueryResult>;
    beginTransaction(): Promise<void>;
    commit(): Promise<void>;
    rollback(): Promise<void>;
    release(): void;
  }
  export interface Pool {
    execute(sql: string, values?: unknown[]): Promise<QueryResult>;
    getConnection(): Promise<PoolConnection>;
  }
}

declare module "better-sqlite3" {
  namespace Database {
    interface Statement {
      get(...params: unknown[]): unknown;
      all(...params: unknown[]): unknown[];
      run(...params: unknown[]): { changes: number; lastInsertRowid: number | bigint };
    }
    interface Database {
      prepare(sql: string): Statement;
      transaction<F extends (...args: never[]) => unknown>(fn: F): F;
    }
  }
  class Database {
    constructor(filename: string, options?: unknown);
  }
  export = Database;
}

declare module "node:sqlite" {
  type SQLOutputValue = null | number | bigint | string | Uint8Array;
  interface StatementResultingChanges {
    changes: number | bigint;
    lastInsertRowid: number | bigint;
  }
  export class StatementSync {
    all(...params: unknown[]): Record<string, SQLOutputValue>[];
    get(...params: unknown[]): Record<string, SQLOutputValue> | undefined;
    run(...params: unknown[]): StatementResultingChanges;
  }
  export class DatabaseSync {
    constructor(path: string, options?: unknown);
    exec(sql: string): void;
    prepare(sql: string): StatementSync;
  }
}
