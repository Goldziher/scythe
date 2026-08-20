// scythe:provenance v=0.17.0 backend=typescript-mysql2 engine=mysql schema=sch2:4332a9c33cb39297 queries=q1:f928696deb211f90 options=opt1:cbf29ce484222325
import type { Pool, RowDataPacket } from "mysql2/promise";


export enum UsersStatus {
	Active = "active",
	Inactive = "inactive",
	Banned = "banned",
}

/** Execute a query returning no rows. */
export async function createOrder(
	pool: Pool,
	user_id: number,
	total: string,
	notes: string | null,
): Promise<void> {
	await pool.execute(
		`INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)`, [user_id, total, notes],
	);
}

/** Row type for GetLastInsertOrder queries. */
export interface GetLastInsertOrderRow extends RowDataPacket {
	id: number;
	user_id: number;
	total: string;
	notes: string | null;
	created_at: Date;
}

/** Fetch a single GetLastInsertOrderRow. */
export async function getLastInsertOrder(
	pool: Pool,
): Promise<GetLastInsertOrderRow> {
	const [rows] = await pool.execute<GetLastInsertOrderRow[]>(
		`SELECT id, user_id, total, notes, created_at FROM orders WHERE id = LAST_INSERT_ID()`,
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetLastInsertOrder");
	}
	return row;
}

/** Row type for GetOrdersByUser queries. */
export interface GetOrdersByUserRow extends RowDataPacket {
	id: number;
	total: string;
	notes: string | null;
	created_at: Date;
}

/** Fetch all GetOrdersByUserRow rows. */
export async function getOrdersByUser(
	pool: Pool,
	user_id: number,
): Promise<GetOrdersByUserRow[]> {
	const [rows] = await pool.execute<GetOrdersByUserRow[]>(
		`SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC`, [user_id],
	);
	return rows;
}

/** Row type for GetOrderTotal queries. */
export interface GetOrderTotalRow extends RowDataPacket {
	total_sum: string | null;
}

/** Fetch a single GetOrderTotalRow. */
export async function getOrderTotal(
	pool: Pool,
	user_id: number,
): Promise<GetOrderTotalRow> {
	const [rows] = await pool.execute<GetOrderTotalRow[]>(
		`SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?`, [user_id],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetOrderTotal");
	}
	return row;
}

/** Execute a query and return the number of affected rows. */
export async function deleteOrdersByUser(
	pool: Pool,
	user_id: number,
): Promise<number> {
	const [result] = await pool.execute(
		`DELETE FROM orders WHERE user_id = ?`, [user_id],
	);
	return (result as { affectedRows: number }).affectedRows;
}

/** Row type for GetUserById queries. */
export interface GetUserByIdRow extends RowDataPacket {
	id: number;
	name: string;
	email: string | null;
	status: UsersStatus;
	created_at: Date;
}

/** Fetch a single GetUserByIdRow. */
export async function getUserById(
	pool: Pool,
	id: number,
): Promise<GetUserByIdRow> {
	const [rows] = await pool.execute<GetUserByIdRow[]>(
		`SELECT id, name, email, status, created_at FROM users WHERE id = ?`, [id],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetUserById");
	}
	return row;
}

/** Row type for ListActiveUsers queries. */
export interface ListActiveUsersRow extends RowDataPacket {
	id: number;
	name: string;
	email: string | null;
}

/** Fetch all ListActiveUsersRow rows. */
export async function listActiveUsers(
	pool: Pool,
	status: UsersStatus,
): Promise<ListActiveUsersRow[]> {
	const [rows] = await pool.execute<ListActiveUsersRow[]>(
		`SELECT id, name, email FROM users WHERE status = ?`, [status],
	);
	return rows;
}

/** Execute a query returning no rows. */
export async function createUser(
	pool: Pool,
	name: string,
	email: string | null,
	status: UsersStatus,
): Promise<void> {
	await pool.execute(
		`INSERT INTO users (name, email, status) VALUES (?, ?, ?)`, [name, email, status],
	);
}

/** Row type for GetLastInsertUser queries. */
export interface GetLastInsertUserRow extends RowDataPacket {
	id: number;
	name: string;
	email: string | null;
	status: UsersStatus;
	created_at: Date;
}

/** Fetch a single GetLastInsertUserRow. */
export async function getLastInsertUser(
	pool: Pool,
): Promise<GetLastInsertUserRow> {
	const [rows] = await pool.execute<GetLastInsertUserRow[]>(
		`SELECT id, name, email, status, created_at FROM users WHERE id = LAST_INSERT_ID()`,
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetLastInsertUser");
	}
	return row;
}

/** Execute a query returning no rows. */
export async function updateUserEmail(
	pool: Pool,
	email: string,
	id: number,
): Promise<void> {
	await pool.execute(
		`UPDATE users SET email = ? WHERE id = ?`, [email, id],
	);
}

/** Execute a query returning no rows. */
export async function deleteUser(pool: Pool, id: number): Promise<void> {
	await pool.execute(
		`DELETE FROM users WHERE id = ?`, [id],
	);
}

/** Row type for SearchUsers queries. */
export interface SearchUsersRow extends RowDataPacket {
	id: number;
	name: string;
	email: string | null;
}

/** Fetch all SearchUsersRow rows. */
export async function searchUsers(
	pool: Pool,
	name: string,
): Promise<SearchUsersRow[]> {
	const [rows] = await pool.execute<SearchUsersRow[]>(
		`SELECT id, name, email FROM users WHERE name LIKE ?`, [name],
	);
	return rows;
}
