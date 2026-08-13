// scythe:provenance v=0.14.0 backend=typescript-kysely engine=mssql schema=sch1:f761f948742217a4 queries=q1:e28b6d666ef6b1da
import { type QueryExecutorProvider, sql } from "kysely";


/** Row type for CreateOrder queries. */
export interface CreateOrderRow {
	id: number;
	user_id: number;
	total: string;
	notes: string | null;
	created_at: Date;
}

/** Fetch a single CreateOrderRow. */
export async function createOrder(
	db: QueryExecutorProvider,
	id: number,
	user_id: number,
	total: string,
	notes: string | null,
): Promise<CreateOrderRow> {
	const result = await sql<CreateOrderRow>`INSERT INTO orders (id, user_id, total, notes)
OUTPUT INSERTED.id, INSERTED.user_id, INSERTED.total, INSERTED.notes, INSERTED.created_at
VALUES (${id}, ${user_id}, ${total}, ${notes})`.execute(db);
	const row = result.rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: CreateOrder");
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
	active: boolean;
	created_at: Date;
}

/** Fetch a single GetUserByIdRow. */
export async function getUserById(
	db: QueryExecutorProvider,
	id: number,
): Promise<GetUserByIdRow> {
	const result = await sql<GetUserByIdRow>`SELECT id, name, email, active, created_at FROM users WHERE id = ${id}`.execute(db);
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
): Promise<ListActiveUsersRow[]> {
	const result = await sql<ListActiveUsersRow>`SELECT id, name, email FROM users WHERE active = CAST(1 AS BIT)`.execute(db);
	return result.rows;
}

/** Row type for CreateUser queries. */
export interface CreateUserRow {
	id: number;
	name: string;
	email: string | null;
	active: boolean;
	created_at: Date;
}

/** Fetch a single CreateUserRow. */
export async function createUser(
	db: QueryExecutorProvider,
	id: number,
	name: string,
	email: string | null,
	active: boolean,
): Promise<CreateUserRow> {
	const result = await sql<CreateUserRow>`INSERT INTO users (id, name, email, active)
OUTPUT INSERTED.id, INSERTED.name, INSERTED.email, INSERTED.active, INSERTED.created_at
VALUES (${id}, ${name}, ${email}, ${active})`.execute(db);
	const row = result.rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: CreateUser");
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
