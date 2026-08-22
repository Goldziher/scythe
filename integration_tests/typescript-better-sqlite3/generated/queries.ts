// scythe:provenance v=0.18.1 backend=typescript-better-sqlite3 engine=sqlite schema=sch2:588fb635332179bc queries=q1:f7199f36438b6396 options=opt1:cbf29ce484222325
import type Database from "better-sqlite3";


/** Execute a query returning no rows. */
export function createOrder(
	db: Database.Database,
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
	db: Database.Database,
	user_id: number,
): GetOrdersByUserRow[] {
	const stmt = db.prepare(`SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC`);
	return stmt.all(user_id) as GetOrdersByUserRow[];
}

/** Row type for GetOrderTotal queries. */
export interface GetOrderTotalRow {
	total_sum: number | null;
}

/** Fetch a single GetOrderTotalRow. */
export function getOrderTotal(
	db: Database.Database,
	user_id: number,
): GetOrderTotalRow {
	const stmt = db.prepare(`SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?`);
	const row = stmt.get(user_id) as GetOrderTotalRow | undefined;
	if (row === undefined) {
		throw new Error("no row found for query: GetOrderTotal");
	}
	return row;
}

/** Execute a query and return the number of affected rows. */
export function deleteOrdersByUser(
	db: Database.Database,
	user_id: number,
): number {
	const stmt = db.prepare(`DELETE FROM orders WHERE user_id = ?`);
	const result = stmt.run(user_id);
	return result.changes;
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
export function getUserById(db: Database.Database, id: number): GetUserByIdRow {
	const stmt = db.prepare(`SELECT id, name, email, status, created_at FROM users WHERE id = ?`);
	const row = stmt.get(id) as GetUserByIdRow | undefined;
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
export function listActiveUsers(
	db: Database.Database,
	status: string,
): ListActiveUsersRow[] {
	const stmt = db.prepare(`SELECT id, name, email FROM users WHERE status = ?`);
	return stmt.all(status) as ListActiveUsersRow[];
}

/** Execute a query returning no rows. */
export function createUser(
	db: Database.Database,
	name: string,
	email: string | null,
	status: string,
): void {
	const stmt = db.prepare(`INSERT INTO users (name, email, status) VALUES (?, ?, ?)`);
	stmt.run(name, email, status);
}

/** Execute a query returning no rows. */
export function updateUserEmail(
	db: Database.Database,
	email: string,
	id: number,
): void {
	const stmt = db.prepare(`UPDATE users SET email = ? WHERE id = ?`);
	stmt.run(email, id);
}

/** Execute a query returning no rows. */
export function deleteUser(db: Database.Database, id: number): void {
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
export function searchUsers(
	db: Database.Database,
	name: string,
): SearchUsersRow[] {
	const stmt = db.prepare(`SELECT id, name, email FROM users WHERE name LIKE ?`);
	return stmt.all(name) as SearchUsersRow[];
}
