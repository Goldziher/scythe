// scythe:provenance v=0.16.1 backend=typescript-wasm-sqlite engine=sqlite schema=sch1:588fb635332179bc queries=q1:f7199f36438b6396 options=opt1:cbf29ce484222325
import type { Database } from "@sqlite.org/sqlite-wasm";


/** Execute a query returning no rows. */
export function createOrder(
	db: Database,
	user_id: number,
	total: number,
	notes: string | null,
): void {
	db.exec({ sql: `INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)`, bind: [user_id, total, notes] });
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
	db: Database,
	user_id: number,
): GetOrdersByUserRow[] {
	return db.selectObjects(`SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC`, [user_id]) as unknown as GetOrdersByUserRow[];
}

/** Row type for GetOrderTotal queries. */
export interface GetOrderTotalRow {
	total_sum: number | null;
}

/** Fetch a single GetOrderTotalRow. */
export function getOrderTotal(db: Database, user_id: number): GetOrderTotalRow {
	const row = db.selectObject(`SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?`, [user_id]) as unknown as GetOrderTotalRow | undefined;
	if (row === undefined) {
		throw new Error("no row found for query: GetOrderTotal");
	}
	return row;
}

/** Execute a query and return the number of affected rows. */
export function deleteOrdersByUser(db: Database, user_id: number): number {
	db.exec({ sql: `DELETE FROM orders WHERE user_id = ?`, bind: [user_id] });
	return db.changes();
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
export function getUserById(db: Database, id: number): GetUserByIdRow {
	const row = db.selectObject(`SELECT id, name, email, status, created_at FROM users WHERE id = ?`, [id]) as unknown as GetUserByIdRow | undefined;
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
	db: Database,
	status: string,
): ListActiveUsersRow[] {
	return db.selectObjects(`SELECT id, name, email FROM users WHERE status = ?`, [status]) as unknown as ListActiveUsersRow[];
}

/** Execute a query returning no rows. */
export function createUser(
	db: Database,
	name: string,
	email: string | null,
	status: string,
): void {
	db.exec({ sql: `INSERT INTO users (name, email, status) VALUES (?, ?, ?)`, bind: [name, email, status] });
}

/** Execute a query returning no rows. */
export function updateUserEmail(db: Database, email: string, id: number): void {
	db.exec({ sql: `UPDATE users SET email = ? WHERE id = ?`, bind: [email, id] });
}

/** Execute a query returning no rows. */
export function deleteUser(db: Database, id: number): void {
	db.exec({ sql: `DELETE FROM users WHERE id = ?`, bind: [id] });
}

/** Row type for SearchUsers queries. */
export interface SearchUsersRow {
	id: number;
	name: string;
	email: string | null;
}

/** Fetch all SearchUsersRow rows. */
export function searchUsers(db: Database, name: string): SearchUsersRow[] {
	return db.selectObjects(`SELECT id, name, email FROM users WHERE name LIKE ?`, [name]) as unknown as SearchUsersRow[];
}
