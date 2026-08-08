// scythe:provenance v=0.14.0 backend=typescript-node-sqlite engine=sqlite schema=sch1:588fb635332179bc
import type { DatabaseSync } from "node:sqlite";


/** Execute a query returning no rows. */
export function createOrder(
	db: DatabaseSync,
	user_id: number,
	total: number,
	notes: string | null,
): void {
	const stmt = db.prepare(`INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)`);
	stmt.run(user_id, total, notes);
}

/** Row type for GetOrdersByUser queries. */
export interface GetOrdersByUserRow {
	id: number;
	total: number;
	notes: string | null;
	created_at: string;
}

/** Fetch all GetOrdersByUserRow rows. */
export function getOrdersByUser(
	db: DatabaseSync,
	user_id: number,
): GetOrdersByUserRow[] {
	const stmt = db.prepare(`SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC`);
	return stmt.all(user_id) as unknown as GetOrdersByUserRow[];
}

/** Row type for GetOrderTotal queries. */
export interface GetOrderTotalRow {
	total_sum: number | null;
}

/** Fetch a single GetOrderTotalRow or null. */
export function getOrderTotal(
	db: DatabaseSync,
	user_id: number,
): GetOrderTotalRow | null {
	const stmt = db.prepare(`SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?`);
	const row = stmt.get(user_id) as GetOrderTotalRow | undefined;
	return row ?? null;
}

/** Execute a query and return the number of affected rows. */
export function deleteOrdersByUser(db: DatabaseSync, user_id: number): number {
	const stmt = db.prepare(`DELETE FROM orders WHERE user_id = ?`);
	const result = stmt.run(user_id);
	return Number(result.changes);
}

/** Row type for GetUserById queries. */
export interface GetUserByIdRow {
	id: number;
	name: string;
	email: string | null;
	status: string;
	created_at: string;
}

/** Fetch a single GetUserByIdRow or null. */
export function getUserById(
	db: DatabaseSync,
	id: number,
): GetUserByIdRow | null {
	const stmt = db.prepare(`SELECT id, name, email, status, created_at FROM users WHERE id = ?`);
	const row = stmt.get(id) as GetUserByIdRow | undefined;
	return row ?? null;
}

/** Row type for ListActiveUsers queries. */
export interface ListActiveUsersRow {
	id: number;
	name: string;
	email: string | null;
}

/** Fetch all ListActiveUsersRow rows. */
export function listActiveUsers(
	db: DatabaseSync,
	status: string,
): ListActiveUsersRow[] {
	const stmt = db.prepare(`SELECT id, name, email FROM users WHERE status = ?`);
	return stmt.all(status) as unknown as ListActiveUsersRow[];
}

/** Execute a query returning no rows. */
export function createUser(
	db: DatabaseSync,
	name: string,
	email: string | null,
	status: string,
): void {
	const stmt = db.prepare(`INSERT INTO users (name, email, status) VALUES (?, ?, ?)`);
	stmt.run(name, email, status);
}

/** Execute a query returning no rows. */
export function updateUserEmail(
	db: DatabaseSync,
	email: string,
	id: number,
): void {
	const stmt = db.prepare(`UPDATE users SET email = ? WHERE id = ?`);
	stmt.run(email, id);
}

/** Execute a query returning no rows. */
export function deleteUser(db: DatabaseSync, id: number): void {
	const stmt = db.prepare(`DELETE FROM users WHERE id = ?`);
	stmt.run(id);
}

/** Row type for SearchUsers queries. */
export interface SearchUsersRow {
	id: number;
	name: string;
	email: string | null;
}

/** Fetch all SearchUsersRow rows. */
export function searchUsers(db: DatabaseSync, name: string): SearchUsersRow[] {
	const stmt = db.prepare(`SELECT id, name, email FROM users WHERE name LIKE ?`);
	return stmt.all(name) as unknown as SearchUsersRow[];
}
