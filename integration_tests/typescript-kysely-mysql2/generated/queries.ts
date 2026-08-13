// scythe:provenance v=0.14.0 backend=typescript-kysely engine=mysql schema=sch1:4332a9c33cb39297 queries=q1:f928696deb211f90
import { type QueryExecutorProvider, sql } from "kysely";


export const UsersStatusValues = {
	Active: "active",
	Inactive: "inactive",
	Banned: "banned",
} as const;

export type UsersStatus = typeof UsersStatusValues[keyof typeof UsersStatusValues];

/** Execute a query returning no rows. */
export async function createOrder(
	db: QueryExecutorProvider,
	user_id: number,
	total: string,
	notes: string | null,
): Promise<void> {
	await sql`INSERT INTO orders (user_id, total, notes) VALUES (${user_id}, ${total}, ${notes})`.execute(db);
}

/** Row type for GetLastInsertOrder queries. */
export interface GetLastInsertOrderRow {
	id: number;
	user_id: number;
	total: string;
	notes: string | null;
	created_at: Date;
}

/** Fetch a single GetLastInsertOrderRow. */
export async function getLastInsertOrder(
	db: QueryExecutorProvider,
): Promise<GetLastInsertOrderRow> {
	const result = await sql<GetLastInsertOrderRow>`SELECT id, user_id, total, notes, created_at FROM orders WHERE id = LAST_INSERT_ID()`.execute(db);
	const row = result.rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetLastInsertOrder");
	}
	return row;
}

/** Row type for GetOrdersByUser queries. */
export interface GetOrdersByUserRow {
	id: number;
	total: string;
	notes: string | null;
	created_at: Date;
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
	total_sum: string | null;
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
	status: UsersStatus;
	created_at: Date;
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
	status: UsersStatus,
): Promise<ListActiveUsersRow[]> {
	const result = await sql<ListActiveUsersRow>`SELECT id, name, email FROM users WHERE status = ${status}`.execute(db);
	return result.rows;
}

/** Execute a query returning no rows. */
export async function createUser(
	db: QueryExecutorProvider,
	name: string,
	email: string | null,
	status: UsersStatus,
): Promise<void> {
	await sql`INSERT INTO users (name, email, status) VALUES (${name}, ${email}, ${status})`.execute(db);
}

/** Row type for GetLastInsertUser queries. */
export interface GetLastInsertUserRow {
	id: number;
	name: string;
	email: string | null;
	status: UsersStatus;
	created_at: Date;
}

/** Fetch a single GetLastInsertUserRow. */
export async function getLastInsertUser(
	db: QueryExecutorProvider,
): Promise<GetLastInsertUserRow> {
	const result = await sql<GetLastInsertUserRow>`SELECT id, name, email, status, created_at FROM users WHERE id = LAST_INSERT_ID()`.execute(db);
	const row = result.rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetLastInsertUser");
	}
	return row;
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
