// scythe:provenance v=0.13.0 backend=typescript-kysely engine=postgresql schema=sch1:2e813606acee8b51
import { type QueryExecutorProvider, sql } from "kysely";


export const UserStatusValues = {
	Active: "active",
	Inactive: "inactive",
	Banned: "banned",
} as const;

export type UserStatus = typeof UserStatusValues[keyof typeof UserStatusValues];

/** Row type for CreateOrder queries. */
export interface CreateOrderRow {
	id: number;
	user_id: number;
	total: string;
	notes: string | null;
	created_at: Date;
}

/** Fetch a single CreateOrderRow or null. */
export async function createOrder(
	db: QueryExecutorProvider,
	user_id: number,
	total: string,
	notes: string | null,
): Promise<CreateOrderRow | null> {
	const result = await sql<CreateOrderRow>`INSERT INTO orders (user_id, total, notes) VALUES (${user_id}, ${total}, ${notes}) RETURNING id, user_id, total, notes, created_at`.execute(db);
	return result.rows[0] ?? null;
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

/** Fetch a single GetOrderTotalRow or null. */
export async function getOrderTotal(
	db: QueryExecutorProvider,
	user_id: number,
): Promise<GetOrderTotalRow | null> {
	const result = await sql<GetOrderTotalRow>`SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ${user_id}`.execute(db);
	return result.rows[0] ?? null;
}

/** Row type for GetOrderWeightTotal queries. */
export interface GetOrderWeightTotalRow {
	weight_total: number | null;
}

/** Fetch a single GetOrderWeightTotalRow or null. */
export async function getOrderWeightTotal(
	db: QueryExecutorProvider,
	user_id: number,
): Promise<GetOrderWeightTotalRow | null> {
	const result = await sql<GetOrderWeightTotalRow>`SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = ${user_id}`.execute(db);
	return result.rows[0] ?? null;
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
	status: UserStatus;
	created_at: Date;
}

/** Fetch a single GetUserByIdRow or null. */
export async function getUserById(
	db: QueryExecutorProvider,
	id: number,
): Promise<GetUserByIdRow | null> {
	const result = await sql<GetUserByIdRow>`SELECT id, name, email, status, created_at FROM users WHERE id = ${id}`.execute(db);
	return result.rows[0] ?? null;
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
	status: UserStatus,
): Promise<ListActiveUsersRow[]> {
	const result = await sql<ListActiveUsersRow>`SELECT id, name, email FROM users WHERE status = ${status}`.execute(db);
	return result.rows;
}

/** Row type for CreateUser queries. */
export interface CreateUserRow {
	id: number;
	name: string;
	email: string | null;
	status: UserStatus;
	created_at: Date;
}

/** Fetch a single CreateUserRow or null. */
export async function createUser(
	db: QueryExecutorProvider,
	name: string,
	email: string | null,
	status: UserStatus,
): Promise<CreateUserRow | null> {
	const result = await sql<CreateUserRow>`INSERT INTO users (name, email, status) VALUES (${name}, ${email}, ${status}) RETURNING id, name, email, status, created_at`.execute(db);
	return result.rows[0] ?? null;
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

/** Row type for GetUserOrders queries. */
export interface GetUserOrdersRow {
	id: number;
	name: string;
	total: string | null;
	notes: string | null;
}

/** Fetch all GetUserOrdersRow rows. */
export async function getUserOrders(
	db: QueryExecutorProvider,
	status: UserStatus,
): Promise<GetUserOrdersRow[]> {
	const result = await sql<GetUserOrdersRow>`SELECT u.id, u.name, o.total, o.notes
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = ${status}`.execute(db);
	return result.rows;
}

/** Row type for CountUsersByStatus queries. */
export interface CountUsersByStatusRow {
	status: UserStatus;
	user_count: number;
}

/** Fetch a single CountUsersByStatusRow or null. */
export async function countUsersByStatus(
	db: QueryExecutorProvider,
	status: UserStatus,
): Promise<CountUsersByStatusRow | null> {
	const result = await sql<CountUsersByStatusRow>`SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = ${status}`.execute(db);
	return result.rows[0] ?? null;
}

/** Row type for GetUserWithTags queries. */
export interface GetUserWithTagsRow {
	id: number;
	name: string;
	tag_name: string;
}

/** Fetch all GetUserWithTagsRow rows. */
export async function getUserWithTags(
	db: QueryExecutorProvider,
	id: number,
): Promise<GetUserWithTagsRow[]> {
	const result = await sql<GetUserWithTagsRow>`SELECT u.id, u.name, t.name AS tag_name
FROM users u
INNER JOIN user_tags ut ON u.id = ut.user_id
INNER JOIN tags t ON ut.tag_id = t.id
WHERE u.id = ${id}`.execute(db);
	return result.rows;
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
