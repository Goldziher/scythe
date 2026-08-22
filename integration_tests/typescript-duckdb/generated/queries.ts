// scythe:provenance v=0.18.1 backend=typescript-duckdb engine=duckdb schema=sch2:a58e9693abcdb5e7 queries=q1:3fcd9a387f9d569e options=opt1:cbf29ce484222325
import type { DuckDBConnection, DuckDBValue } from "@duckdb/node-api";

function firstRow<T>(rows: readonly unknown[]): T | null {
	return rows.length === 0 ? null : (rows[0] as T);
}

function allRows<T>(rows: readonly unknown[]): T[] {
	return rows as T[];
}


/** Execute a query returning no rows. */
export async function createOrder(
	conn: DuckDBConnection,
	user_id: number,
	total: number,
	notes: string | null,
): Promise<void> {
	const stmt = await conn.prepare(
		`INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3)`,
	);
	stmt.bind([user_id, total, notes] as DuckDBValue[]);
	await stmt.run();
}

/** Row type for GetOrdersByUser queries. */
export interface GetOrdersByUserRow {
	id: number;
	total: number;
	notes: string | null;
	created_at: string;
}

/** Fetch all GetOrdersByUserRow rows. */
export async function getOrdersByUser(
	conn: DuckDBConnection,
	user_id: number,
): Promise<GetOrdersByUserRow[]> {
	const stmt = await conn.prepare(
		`SELECT id, total, notes, created_at FROM orders WHERE user_id = $1 ORDER BY created_at DESC`,
	);
	stmt.bind([user_id] as DuckDBValue[]);
	const result = await stmt.run();
	return allRows<GetOrdersByUserRow>(await result.getRowObjects());
}

/** Row type for GetOrderTotal queries. */
export interface GetOrderTotalRow {
	total_sum: number | null;
}

/** Fetch a single GetOrderTotalRow. */
export async function getOrderTotal(
	conn: DuckDBConnection,
	user_id: number,
): Promise<GetOrderTotalRow> {
	const stmt = await conn.prepare(
		`SELECT SUM(total) AS total_sum FROM orders WHERE user_id = $1`,
	);
	stmt.bind([user_id] as DuckDBValue[]);
	const result = await stmt.run();
	const rows = await result.getRowObjects();
	const row = firstRow<GetOrderTotalRow>(rows);
	if (row === null) {
		throw new Error("no row found for query: GetOrderTotal");
	}
	return row;
}

/** Execute a query and return the number of affected rows. */
export async function deleteOrdersByUser(
	conn: DuckDBConnection,
	user_id: number,
): Promise<number> {
	const stmt = await conn.prepare(`DELETE FROM orders WHERE user_id = $1`);
	stmt.bind([user_id] as DuckDBValue[]);
	const result = await stmt.run();
	return result.rowsChanged;
}

/** Row type for GetUserById queries. */
export interface GetUserByIdRow {
	id: number;
	name: string;
	email: string | null;
	status: string;
	created_at: string;
}

/** Fetch a single GetUserByIdRow. */
export async function getUserById(
	conn: DuckDBConnection,
	id: number,
): Promise<GetUserByIdRow> {
	const stmt = await conn.prepare(
		`SELECT id, name, email, status, created_at FROM users WHERE id = $1`,
	);
	stmt.bind([id] as DuckDBValue[]);
	const result = await stmt.run();
	const rows = await result.getRowObjects();
	const row = firstRow<GetUserByIdRow>(rows);
	if (row === null) {
		throw new Error("no row found for query: GetUserById");
	}
	return row;
}

/** Row type for ListActiveUsers queries. */
export interface ListActiveUsersRow {
	id: number;
	name: string;
	email: string | null;
}

/** Fetch all ListActiveUsersRow rows. */
export async function listActiveUsers(
	conn: DuckDBConnection,
	status: string,
): Promise<ListActiveUsersRow[]> {
	const stmt = await conn.prepare(
		`SELECT id, name, email FROM users WHERE status = $1`,
	);
	stmt.bind([status] as DuckDBValue[]);
	const result = await stmt.run();
	return allRows<ListActiveUsersRow>(await result.getRowObjects());
}

/** Execute a query returning no rows. */
export async function createUser(
	conn: DuckDBConnection,
	name: string,
	email: string | null,
	status: string,
): Promise<void> {
	const stmt = await conn.prepare(
		`INSERT INTO users (name, email, status) VALUES ($1, $2, $3)`,
	);
	stmt.bind([name, email, status] as DuckDBValue[]);
	await stmt.run();
}

/** Execute a query returning no rows. */
export async function updateUserEmail(
	conn: DuckDBConnection,
	email: string,
	id: number,
): Promise<void> {
	const stmt = await conn.prepare(`UPDATE users SET email = $1 WHERE id = $2`);
	stmt.bind([email, id] as DuckDBValue[]);
	await stmt.run();
}

/** Execute a query returning no rows. */
export async function deleteUser(
	conn: DuckDBConnection,
	id: number,
): Promise<void> {
	const stmt = await conn.prepare(`DELETE FROM users WHERE id = $1`);
	stmt.bind([id] as DuckDBValue[]);
	await stmt.run();
}

/** Row type for SearchUsers queries. */
export interface SearchUsersRow {
	id: number;
	name: string;
	email: string | null;
}

/** Fetch all SearchUsersRow rows. */
export async function searchUsers(
	conn: DuckDBConnection,
	name: string,
): Promise<SearchUsersRow[]> {
	const stmt = await conn.prepare(
		`SELECT id, name, email FROM users WHERE name LIKE $1`,
	);
	stmt.bind([name] as DuckDBValue[]);
	const result = await stmt.run();
	return allRows<SearchUsersRow>(await result.getRowObjects());
}
