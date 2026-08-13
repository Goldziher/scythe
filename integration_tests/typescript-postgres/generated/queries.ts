// scythe:provenance v=0.14.0 backend=typescript-postgres engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:03c2db16665ee046
import type { Sql } from "postgres";


export enum UserStatus {
	Active = "active",
	Inactive = "inactive",
	Banned = "banned",
}

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
    INSERT INTO orders (user_id, total, notes) VALUES (${user_id}, ${total}, ${notes}) RETURNING id, user_id, total, notes, created_at
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

/** Row type for GetOrderWeightTotal queries. */
export interface GetOrderWeightTotalRow {
	weight_total: number | null;
}

/** Fetch a single GetOrderWeightTotalRow. */
export async function getOrderWeightTotal(
	sql: Sql,
	user_id: number,
): Promise<GetOrderWeightTotalRow> {
	const rows = await sql<GetOrderWeightTotalRow[]>`
    SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = ${user_id}
  `;
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetOrderWeightTotal");
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
	status: UserStatus;
	created_at: Date;
}

/** Fetch a single GetUserByIdRow. */
export async function getUserById(
	sql: Sql,
	id: number,
): Promise<GetUserByIdRow> {
	const rows = await sql<GetUserByIdRow[]>`
    SELECT id, name, email, status, created_at FROM users WHERE id = ${id}
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
	status: UserStatus,
): Promise<ListActiveUsersRow[]> {
	const rows = await sql<ListActiveUsersRow[]>`
    SELECT id, name, email FROM users WHERE status = ${status}
  `;
	return rows;
}

/** Row type for CreateUser queries. */
export interface CreateUserRow {
	id: number;
	name: string;
	email: string | null;
	status: UserStatus;
	created_at: Date;
}

/** Fetch a single CreateUserRow. */
export async function createUser(
	sql: Sql,
	name: string,
	email: string | null,
	status: UserStatus,
): Promise<CreateUserRow> {
	const rows = await sql<CreateUserRow[]>`
    INSERT INTO users (name, email, status) VALUES (${name}, ${email}, ${status}) RETURNING id, name, email, status, created_at
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

/** Row type for GetUserOrders queries. */
export interface GetUserOrdersRow {
	id: number;
	name: string;
	total: string | null;
	notes: string | null;
}

/** Fetch all GetUserOrdersRow rows. */
export async function getUserOrders(
	sql: Sql,
	status: UserStatus,
): Promise<GetUserOrdersRow[]> {
	const rows = await sql<GetUserOrdersRow[]>`
    SELECT u.id, u.name, o.total, o.notes
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = ${status}
  `;
	return rows;
}

/** Row type for CountUsersByStatus queries. */
export interface CountUsersByStatusRow {
	status: UserStatus;
	user_count: number;
}

/** Fetch a single CountUsersByStatusRow. */
export async function countUsersByStatus(
	sql: Sql,
	status: UserStatus,
): Promise<CountUsersByStatusRow> {
	const rows = await sql<CountUsersByStatusRow[]>`
    SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = ${status}
  `;
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: CountUsersByStatus");
	}
	return row;
}

/** Row type for GetUserWithTags queries. */
export interface GetUserWithTagsRow {
	id: number;
	name: string;
	tag_name: string;
}

/** Fetch all GetUserWithTagsRow rows. */
export async function getUserWithTags(
	sql: Sql,
	id: number,
): Promise<GetUserWithTagsRow[]> {
	const rows = await sql<GetUserWithTagsRow[]>`
    SELECT u.id, u.name, t.name AS tag_name
FROM users u
INNER JOIN user_tags ut ON u.id = ut.user_id
INNER JOIN tags t ON ut.tag_id = t.id
WHERE u.id = ${id}
  `;
	return rows;
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
	name: string,
): Promise<SearchUsersRow[]> {
	const rows = await sql<SearchUsersRow[]>`
    SELECT id, name, email FROM users WHERE name LIKE ${name}
  `;
	return rows;
}
