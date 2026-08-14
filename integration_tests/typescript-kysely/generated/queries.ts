// scythe:provenance v=0.15.0 backend=typescript-kysely engine=postgresql schema=sch1:c247390d575b8f71 queries=q1:a78685f58b075ff5 options=opt1:cbf29ce484222325
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

/** Fetch a single CreateOrderRow. */
export async function createOrder(
	db: QueryExecutorProvider,
	user_id: number,
	total: string,
	notes: string | null,
): Promise<CreateOrderRow> {
	const result = await sql<CreateOrderRow>`INSERT INTO orders (user_id, total, notes) VALUES (${user_id}, ${total}, ${notes}) RETURNING id, user_id, total, notes, created_at`.execute(db);
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

/** Row type for GetOrderWeightTotal queries. */
export interface GetOrderWeightTotalRow {
	weight_total: number | null;
}

/** Fetch a single GetOrderWeightTotalRow. */
export async function getOrderWeightTotal(
	db: QueryExecutorProvider,
	user_id: number,
): Promise<GetOrderWeightTotalRow> {
	const result = await sql<GetOrderWeightTotalRow>`SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = ${user_id}`.execute(db);
	const row = result.rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetOrderWeightTotal");
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
	status: UserStatus;
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

/** Fetch a single CreateUserRow. */
export async function createUser(
	db: QueryExecutorProvider,
	name: string,
	email: string | null,
	status: UserStatus,
): Promise<CreateUserRow> {
	const result = await sql<CreateUserRow>`INSERT INTO users (name, email, status) VALUES (${name}, ${email}, ${status}) RETURNING id, name, email, status, created_at`.execute(db);
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

/** Fetch a single CountUsersByStatusRow. */
export async function countUsersByStatus(
	db: QueryExecutorProvider,
	status: UserStatus,
): Promise<CountUsersByStatusRow> {
	const result = await sql<CountUsersByStatusRow>`SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = ${status}`.execute(db);
	const row = result.rows[0];
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

/** Composite type user_address. */
export interface UserAddress {
	street: string;
	city: string;
	zip: string;
}

/** Row type for GetUserProfile queries. */
export interface GetUserProfileRow {
	id: number;
	secondary_status: UserStatus | null;
	address: UserAddress | null;
}

/** Fetch a single GetUserProfileRow. */
export async function getUserProfile(
	db: QueryExecutorProvider,
	id: number,
): Promise<GetUserProfileRow> {
	const result = await sql<GetUserProfileRow>`SELECT id, secondary_status, address FROM users WHERE id = ${id}`.execute(db);
	const row = result.rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetUserProfile");
	}
	return row;
}
