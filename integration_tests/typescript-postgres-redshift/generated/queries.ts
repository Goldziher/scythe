// scythe:provenance v=0.16.1 backend=typescript-postgres engine=redshift schema=sch2:a4457eae974a6707 queries=q1:1d594d539783fc08 options=opt1:cbf29ce484222325
type PostgresJsonValue = null | string | number | boolean | Date | readonly PostgresJsonValue[] | { readonly [key: string]: undefined | PostgresJsonValue };
import type { Sql } from "postgres";


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
	sql: Sql,
	user_id: number,
	total: string,
	notes: string | null,
): Promise<CreateOrderRow> {
	const rows = await sql<CreateOrderRow[]>`
    INSERT INTO orders (user_id, total, notes)
VALUES (${user_id}, ${total}, ${notes})
RETURNING id, user_id, total, notes, created_at
  `;
	const row = rows[0];
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
	sql: Sql,
	user_id: number,
): Promise<GetOrdersByUserRow[]> {
	const rows = await sql<GetOrdersByUserRow[]>`
    SELECT id, total, notes, created_at FROM orders WHERE user_id = ${user_id} ORDER BY created_at DESC
  `;
	return rows;
}

/** Row type for GetOrderTotal queries. */
export interface GetOrderTotalRow {
	total_sum: string | null;
}

/** Fetch a single GetOrderTotalRow. */
export async function getOrderTotal(
	sql: Sql,
	user_id: number,
): Promise<GetOrderTotalRow> {
	const rows = await sql<GetOrderTotalRow[]>`
    SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ${user_id}
  `;
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetOrderTotal");
	}
	return row;
}

/** Execute a query and return the number of affected rows. */
export async function deleteOrdersByUser(
	sql: Sql,
	user_id: number,
): Promise<number> {
	const result = await sql`
    DELETE FROM orders WHERE user_id = ${user_id}
  `;
	return result.count;
}

/** Row type for GetUserById queries. */
export interface GetUserByIdRow {
	id: number;
	name: string;
	email: string | null;
	status: string;
	created_at: Date;
}

/** Fetch a single GetUserByIdRow. */
export async function getUserById(
	sql: Sql,
	id: number,
): Promise<GetUserByIdRow> {
	const rows = await sql<GetUserByIdRow[]>`
    SELECT id, name, email, status, created_at
FROM users
WHERE id = ${id}
  `;
	const row = rows[0];
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
	sql: Sql,
	status: string,
): Promise<ListActiveUsersRow[]> {
	const rows = await sql<ListActiveUsersRow[]>`
    SELECT id, name, email
FROM users
WHERE status = ${status}
  `;
	return rows;
}

/** Row type for CreateUser queries. */
export interface CreateUserRow {
	id: number;
	name: string;
	email: string | null;
	status: string;
	created_at: Date;
}

/** Fetch a single CreateUserRow. */
export async function createUser(
	sql: Sql,
	name: string,
	email: string | null,
	status: string,
): Promise<CreateUserRow> {
	const rows = await sql<CreateUserRow[]>`
    INSERT INTO users (name, email, status)
VALUES (${name}, ${email}, ${status})
RETURNING id, name, email, status, created_at
  `;
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: CreateUser");
	}
	return row;
}

/** Execute a query returning no rows. */
export async function updateUserEmail(
	sql: Sql,
	email: string,
	id: number,
): Promise<void> {
	await sql`
    UPDATE users SET email = ${email} WHERE id = ${id}
  `;
}

/** Execute a query returning no rows. */
export async function deleteUser(sql: Sql, id: number): Promise<void> {
	await sql`
    DELETE FROM users WHERE id = ${id}
  `;
}

/** Row type for SearchUsers queries. */
export interface SearchUsersRow {
	id: number;
	name: string;
	email: string | null;
}

/** Fetch all SearchUsersRow rows. */
export async function searchUsers(
	sql: Sql,
	status: string,
): Promise<SearchUsersRow[]> {
	const rows = await sql<SearchUsersRow[]>`
    SELECT id, name, email
FROM users
WHERE status = ${status}
ORDER BY name
  `;
	return rows;
}
