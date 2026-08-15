// scythe:provenance v=0.16.0 backend=typescript-postgres engine=postgresql schema=sch1:c247390d575b8f71 queries=q1:a78685f58b075ff5 options=opt1:cbf29ce484222325
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

/** Composite type user_address. */
export interface UserAddress {
	street: string;
	city: string;
	zip: string;
}

// ~keep board #204: postgres.js has no adapter for a user-defined composite -- it
// hands back the driver's raw text form as a plain string. Parse it here.
export function parseUserAddress(raw: unknown): UserAddress | null {
	if (raw === null || raw === undefined) {
		return null;
	}
	const f = parseUserAddressFields(raw as string);
	return {
		street: f[0] as string,
		city: f[1] as string,
		zip: f[2] as string,
	};
}

function parseUserAddressFields(text: string): (string | null)[] {
	// ~keep Splits a PostgreSQL composite's text form ("(a,b,c)") into its raw field
	// tokens, honoring its escaping rules: an empty unquoted field is SQL NULL (returned as
	// null); a field needing quoting (comma, paren, quote, backslash, leading/trailing
	// space, or the empty string) is wrapped in double quotes; every other field is
	// unquoted and taken literally. Inside a quoted field `record_out` writes a literal
	// '"' as '""' and a literal '\\' as '\\\\' -- reading '""' as a closing quote both
	// truncates the value and desynchronizes every field after it. Verified against
	// PostgreSQL 16.
	const fields: (string | null)[] = [];
	const inner = text.slice(1, -1);
	let i = 0;
	const n = inner.length;
	for (;;) {
		let chars = "";
		let isNull = false;
		if (i < n && inner[i] === '"') {
			i++;
			while (i < n) {
				const c = inner[i];
				if (c === "\\" && i + 1 < n) {
					chars += inner[i + 1];
					i += 2;
				} else if (c === '"' && i + 1 < n && inner[i + 1] === '"') {
					chars += '"';
					i += 2;
				} else if (c === '"') {
					i++;
					break;
				} else {
					chars += c;
					i++;
				}
			}
		} else {
			const start = i;
			while (i < n && inner[i] !== ",") {
				i++;
			}
			chars = inner.slice(start, i);
			isNull = chars.length === 0;
		}
		fields.push(isNull ? null : chars);
		if (i < n && inner[i] === ",") {
			i++;
			continue;
		}
		break;
	}
	return fields;
}

/** Row type for GetUserProfile queries. */
export interface GetUserProfileRow {
	id: number;
	secondary_status: UserStatus | null;
	address: UserAddress | null;
}

/** Fetch a single GetUserProfileRow. */
export async function getUserProfile(
	sql: Sql,
	id: number,
): Promise<GetUserProfileRow> {
	const rows = await sql<GetUserProfileRow[]>`
    SELECT id, secondary_status, address FROM users WHERE id = ${id}
  `;
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetUserProfile");
	}
	return {
		...row,
		address: parseUserAddress(row['address']) as UserAddress | null,
	};
}
