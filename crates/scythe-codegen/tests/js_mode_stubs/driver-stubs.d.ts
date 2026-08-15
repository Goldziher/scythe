// ~keep Hand-written ambient stubs for the driver packages the `javascript-*`
// (JSDoc emit mode, #81/#93) backends reference via `import("pkg").Type`
// JSDoc annotations: `pg`, `postgres`, `mysql2/promise`, `better-sqlite3`,
// `node:sqlite`, `@sqlite.org/sqlite-wasm`, `snowflake-sdk`, `@duckdb/node-api`,
// `oracledb`, `mssql`.
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
//   - `@sqlite.org/sqlite-wasm`'s `Database.selectObject`/`.selectObjects`
//     return `Record<string, SqlValue> | undefined` / `Record<string,
//     SqlValue>[]` (copied from `@sqlite.org/sqlite-wasm@3.53.0-build1`'s
//     `src/index.d.ts`). Both the single-row and the array cast are genuine
//     TS2352s as `as`, but the JSDoc spelling of both is accepted -- so
//     `javascript-wasm-sqlite` casts in one step everywhere too, unlike the
//     TS backend a few lines up in `typescript_wasm_sqlite.rs`, which routes
//     both through `as unknown as`.
//   - `snowflake-sdk`'s `Connection.execute`'s `complete` callback hands back
//     `rows?: Array<any>` (copied from `snowflake-sdk@3.1.0`'s `index.d.ts`),
//     so no row cast is a genuine TS2352 for `javascript-snowflake` either --
//     but `binds: Binds` (`Binds = readonly Bind[] | InsertBinds`, `Bind =
//     string | number | boolean | null`) does not admit every
//     `ResolvedParam::full_type` this backend emits (`Date`, `Buffer`, ...),
//     and a bound array containing one of those *is* a genuine TS2352 as a
//     single-step JSDoc cast, exactly as it is as `as Binds` -- so
//     `typescript_snowflake.rs::generate_query_fn_js` routes `binds` through
//     an explicit `unknown` hop, the one JSDoc cast in this file that still
//     needs one.
//   - `@duckdb/node-api`'s `DuckDBResult.getRowObjects()` (inherited by the
//     `DuckDBMaterializedResult` `DuckDBPreparedStatement.run()` returns)
//     declares `Promise<Record<string, DuckDBValue>[]>` (copied from
//     `@duckdb/node-api@1.5.5-r.4`'s `lib/DuckDBResult.d.ts`/`values/DuckDBValue.d.ts`,
//     verified by installing the real package, not approximated) -- a
//     concrete record type, not `unknown`, so `javascript-duckdb`'s row cast
//     is a genuine TS2352 as `as` (which is why the TS backend funnels
//     through `firstRow<T>`/`allRows<T>`, both typed `readonly unknown[]`),
//     but the JSDoc spelling of the identical direct assertion is accepted,
//     so `javascript-duckdb` casts in one step everywhere, same as
//     `javascript-wasm-sqlite`.
//   - `oracledb`'s `Connection.execute<T>` (copied from
//     `@types/oracledb@7.0.2`'s `index.d.ts`) is called with no explicit
//     type argument by both the TS and JS backends, so `T` has nothing to
//     infer from and resolves to `unknown`: `Result<unknown>.rows` /
//     `.outBinds` are `unknown[] | undefined` / `unknown | undefined`. A
//     single-step assertion off `unknown` is always accepted by both `as`
//     and the JSDoc inline cast, so `javascript-oracledb` needs no
//     `unknown`-hop question at all -- unlike `javascript-duckdb` above,
//     whose read is concrete, not `unknown`.
//   - `mssql`'s `Request.query<Entity>` (copied from `@types/mssql@12.3.0`'s
//     `index.d.ts`) *is* called with an explicit `<Entity>` type argument by
//     the TS backend (`request.query<GetSessionRow>(...)`), which plain
//     JSDoc has no syntax to spell at a call site at all. Falling back to
//     the same package's non-generic overload (`query(command):
//     Promise<IResult<any>>`) makes every read `any`, and `any` needs no
//     cast to flow into a concrete type -- so `javascript-mssql` is the one
//     backend in this file whose row read carries no JSDoc cast whatsoever,
//     confirmed against this stub rather than assumed.
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

declare module "@sqlite.org/sqlite-wasm" {
  type SqlValue = string | number | null | bigint | Uint8Array | Int8Array | ArrayBuffer;
  export class Database {
    selectObject(sql: string, bind?: unknown): Record<string, SqlValue> | undefined;
    selectObjects(sql: string, bind?: unknown): Record<string, SqlValue>[];
    exec(sql: string, opts?: { bind?: unknown }): this;
    exec(opts: { sql: string; bind?: unknown }): this;
    changes(): number;
    transaction<T>(callback: (db: this) => T): T;
  }
}

declare module "snowflake-sdk" {
  type Bind = string | number | boolean | null;
  type InsertBinds = readonly Bind[][];
  export type Binds = readonly Bind[] | InsertBinds;
  type StatementCallback = (err: Error | undefined, stmt: RowStatement, rows?: Array<any> | undefined) => void;
  export interface RowStatement {
    getNumUpdatedRows(): number | undefined;
  }
  export interface Connection {
    execute(options: { sqlText: string; binds?: Binds; complete?: StatementCallback }): RowStatement;
  }
}

declare module "@duckdb/node-api" {
  export class DuckDBBlobValue {
    readonly bytes: Uint8Array;
    constructor(bytes: Uint8Array);
  }
  export type DuckDBValue = null | boolean | number | bigint | string | DuckDBBlobValue;
  export class DuckDBMaterializedResult {
    getRowObjects(): Promise<Record<string, DuckDBValue>[]>;
    readonly rowsChanged: number;
  }
  export class DuckDBPreparedStatement {
    bind(values: DuckDBValue[] | Record<string, DuckDBValue>): void;
    run(): Promise<DuckDBMaterializedResult>;
  }
  export class DuckDBConnection {
    prepare(sql: string): Promise<DuckDBPreparedStatement>;
  }
}

declare module "oracledb" {
  namespace OracleDB {
    const OUT_FORMAT_OBJECT: number;
    const BIND_OUT: number;
    const NUMBER: number;
    const DATE: number;
    const STRING: number;

    type BindParameter = { dir?: number; type?: number; val?: unknown };
    type BindParameters =
      | Record<string, BindParameter | string | number | bigint | boolean | Date | null | undefined>
      | Array<BindParameter | string | number | bigint | boolean | Date | null | undefined>;

    interface ExecuteOptions {
      outFormat?: number;
      [key: string]: unknown;
    }

    interface Result<T> {
      outBinds?: T | undefined;
      rows?: T[] | undefined;
      rowsAffected?: number | undefined;
    }
    interface Results<T> {
      outBinds?: T[] | undefined;
      rowsAffected?: number | undefined;
    }

    interface Connection {
      execute<T>(sql: string, bindParams: BindParameters, options: ExecuteOptions): Promise<Result<T>>;
      execute<T>(sql: string, bindParams: BindParameters): Promise<Result<T>>;
      execute<T>(sql: string): Promise<Result<T>>;
      executeMany<T>(sql: string, binds: BindParameters[]): Promise<Results<T>>;
      executeMany<T>(sql: string, iterations: number): Promise<Results<T>>;
    }
  }
  export = OracleDB;
}

declare module "mssql" {
  // ~keep Unlike the `oracledb` block above (a `namespace` wrapped in
  // `export =`), this one mirrors the real `@types/mssql@12.3.0` package's
  // own shape more directly: plain top-level `export`s, no `export =` and no
  // explicit `export default`. Verified against the real package (`npm
  // install @types/mssql`, not approximated): with no `package.json`
  // anywhere above this file declaring `"type": "module"` (confirmed for
  // this repo -- there is none between here and the filesystem root), the
  // real `mssql` runtime package is CommonJS, and `--module nodenext`
  // synthesizes `import sql from "mssql"`'s default binding as the whole
  // export table -- which is what makes `sql.ConnectionPool` resolve as a
  // *type* position, not just `sql.Int` as a value. An earlier draft of this
  // stub wrapped everything in an explicit `const sql = { ... }; export
  // default sql;` object literal instead, which compiles but produces a
  // plain value type with no merged namespace, and `@param
  // {sql.ConnectionPool}` failed with `TS2503: Cannot find namespace 'sql'`
  // -- caught by trying it against this exact stub before shipping it.
  interface ISqlTypeFactory {
    (): unknown;
  }
  export const SmallInt: ISqlTypeFactory;
  export const Int: ISqlTypeFactory;
  export const BigInt: ISqlTypeFactory;
  export const Real: ISqlTypeFactory;
  export const Float: ISqlTypeFactory;
  export const VarChar: ISqlTypeFactory;
  export const Bit: ISqlTypeFactory;
  export const NVarChar: ISqlTypeFactory;
  export const Text: ISqlTypeFactory;
  export const Date: ISqlTypeFactory;
  export const DateTime: ISqlTypeFactory;
  export const DateTimeOffset: ISqlTypeFactory;
  export const UniqueIdentifier: ISqlTypeFactory;
  export const Binary: ISqlTypeFactory;

  export interface IResult<T> {
    recordset: IRecordSet<T>;
    rowsAffected: number[];
  }
  export interface IRecordSet<T> extends Array<T> {}

  export class Request {
    input(name: string, value: unknown): Request;
    input(name: string, type: unknown, value: unknown): Request;
    query(command: string): Promise<IResult<any>>;
    query<Entity>(command: string): Promise<IResult<Entity>>;
  }
  export class Transaction {
    begin(): Promise<void>;
    commit(): Promise<void>;
    rollback(): Promise<void>;
    request(): Request;
  }
  export class ConnectionPool {
    request(): Request;
    transaction(): Transaction;
  }
}
