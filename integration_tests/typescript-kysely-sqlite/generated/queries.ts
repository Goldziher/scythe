// scythe:provenance v=0.14.0 backend=typescript-kysely engine=sqlite schema=sch1:588fb635332179bc queries=q1:f7199f36438b6396
import { type QueryExecutorProvider, sql } from "kysely";


/** Execute a query returning no rows. */
export async function createOrder(
	db: QueryExecutorProvider,
	user_id: number,
	total: number,
	notes: string | null,
): Promise<void> {
	await sql`INSERT INTO orders (user_id, total, notes) VALUES (${user_id}, ${total}, ${notes})`.execute(db);
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
	db: QueryExecutorProvider,
	user_id: number,
): Promise<GetOrdersByUserRow[]> {
	const result = await sql<GetOrdersByUserRow>`SELECT id, total, notes, created_at FROM orders WHERE user_id = ${user_id} ORDER BY created_at DESC`.execute(db);
	return result.rows;
}

/** Row type for GetOrderTotal queries. */
export interface GetOrderTotalRow {
	total_sum: number | null;
}

/** Fetch a single GetOrderTotalRow. */
export async function getOrderTotal(
	db: QueryExecutorProvider,
	user_id: number,
): Promise<GetOrderTotalRow> {
	const result = await sql<GetOrderTotalRow>`SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ${user_id}`.execute(db);
	const row = result.rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetOrderTotal");
	}
	return row;
}

/** Execute a query and return the number of affected rows. */
export async function deleteOrdersByUser(
	db: QueryExecutorProvider,
	user_id: number,
): Promise<number> {
	const result = await sql`DELETE FROM orders WHERE user_id = ${user_id}`.execute(db);
	return Number(result.numAffectedRows ?? 0n);
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
	db: QueryExecutorProvider,
	id: number,
): Promise<GetUserByIdRow> {
	const result = await sql<GetUserByIdRow>`SELECT id, name, email, status, created_at FROM users WHERE id = ${id}`.execute(db);
	const row = result.rows[0];
	if (row === undefined) {
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
	db: QueryExecutorProvider,
	status: string,
): Promise<ListActiveUsersRow[]> {
	const result = await sql<ListActiveUsersRow>`SELECT id, name, email FROM users WHERE status = ${status}`.execute(db);
	return result.rows;
}

/** Execute a query returning no rows. */
export async function createUser(
	db: QueryExecutorProvider,
	name: string,
	email: string | null,
	status: string,
): Promise<void> {
	await sql`INSERT INTO users (name, email, status) VALUES (${name}, ${email}, ${status})`.execute(db);
}

/** Execute a query returning no rows. */
export async function updateUserEmail(
	db: QueryExecutorProvider,
	email: string,
	id: number,
): Promise<void> {
	await sql`UPDATE users SET email = ${email} WHERE id = ${id}`.execute(db);
}

/** Execute a query returning no rows. */
export async function deleteUser(
	db: QueryExecutorProvider,
	id: number,
): Promise<void> {
	await sql`DELETE FROM users WHERE id = ${id}`.execute(db);
}

/** Row type for SearchUsers queries. */
export interface SearchUsersRow {
	id: number;
	name: string;
	email: string | null;
}

/** Fetch all SearchUsersRow rows. */
export async function searchUsers(
	db: QueryExecutorProvider,
	name: string,
): Promise<SearchUsersRow[]> {
	const result = await sql<SearchUsersRow>`SELECT id, name, email FROM users WHERE name LIKE ${name}`.execute(db);
	return result.rows;
}
