// scythe:provenance v=0.18.0 backend=typescript-mysql2 engine=mariadb schema=sch2:262bec5a0954c973 queries=q1:2f37bd0f0a685c79 options=opt1:cbf29ce484222325
import type { Pool, RowDataPacket } from "mysql2/promise";


export enum UsersStatus {
	Active = "active",
	Inactive = "inactive",
	Banned = "banned",
}

/** Row type for CreateOrder queries. */
export interface CreateOrderRow extends RowDataPacket {
	id: number;
	user_id: string;
	total: string;
	notes: string | null;
	created_at: Date;
}

/** Fetch a single CreateOrderRow. */
export async function createOrder(
	pool: Pool,
	user_id: string,
	total: string,
	notes: string | null,
): Promise<CreateOrderRow> {
	const [rows] = await pool.execute<CreateOrderRow[]>(
		`INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at`, [user_id, total, notes],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: CreateOrder");
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
	user_id: string,
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
	user_id: string,
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
	user_id: string,
): Promise<number> {
	const [result] = await pool.execute(
		`DELETE FROM orders WHERE user_id = ?`, [user_id],
	);
	return (result as { affectedRows: number }).affectedRows;
}

/** Row type for GetUserById queries. */
export interface GetUserByIdRow extends RowDataPacket {
	id: string;
	name: string;
	email: string | null;
	status: UsersStatus;
	created_at: Date;
}

/** Fetch a single GetUserByIdRow. */
export async function getUserById(
	pool: Pool,
	id: string,
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
	id: string;
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

/** Row type for CreateUser queries. */
export interface CreateUserRow extends RowDataPacket {
	id: string;
	name: string;
	email: string | null;
}

/** Fetch a single CreateUserRow. */
export async function createUser(
	pool: Pool,
	name: string,
	email: string | null,
	status: UsersStatus,
): Promise<CreateUserRow> {
	const [rows] = await pool.execute<CreateUserRow[]>(
		`INSERT INTO users (name, email, status) VALUES (?, ?, ?) RETURNING id, name, email`, [name, email, status],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: CreateUser");
	}
	return row;
}

/** Execute a query returning no rows. */
export async function updateUserEmail(
	pool: Pool,
	email: string,
	id: string,
): Promise<void> {
	await pool.execute(
		`UPDATE users SET email = ? WHERE id = ?`, [email, id],
	);
}

/** Execute a query returning no rows. */
export async function deleteUser(pool: Pool, id: string): Promise<void> {
	await pool.execute(
		`DELETE FROM users WHERE id = ? RETURNING id`, [id],
	);
}

/** Row type for SearchUsers queries. */
export interface SearchUsersRow extends RowDataPacket {
	id: string;
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
