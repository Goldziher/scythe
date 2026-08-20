// scythe:provenance v=0.17.0 backend=typescript-pg engine=postgresql schema=sch2:c247390d575b8f71 queries=q1:b6aca93cc722fe32 options=opt1:304531517b9a94ef
import type { PoolClient } from "pg";


export const UserStatusValues = {
	Active: "active",
	Inactive: "inactive",
	Banned: "banned",
} as const;

export type UserStatus = typeof UserStatusValues[keyof typeof UserStatusValues];

/** Row type for CreateOrder queries. */
export interface CreateOrderRow {
	id: number;
	userId: number;
	total: string;
	notes: string | null;
	createdAt: Date;
}

/** Fetch a single CreateOrderRow. */
export async function createOrder(
	client: PoolClient,
	userId: number,
	total: string,
	notes: string | null,
): Promise<CreateOrderRow> {
	const { rows } = await client.query<Record<string, unknown>>(
		`INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3) RETURNING id, user_id, total, notes, created_at`,
		[userId, total, notes],
	);
	const row = rows[0];
	if (!row) {
		throw new Error("no row found for query: CreateOrder");
	}
	return {
		id: row.id as number,
		userId: row.user_id as number,
		total: row.total as string,
		notes: row.notes as string | null,
		createdAt: row.created_at as Date,
	};
}

/** Row type for GetOrdersByUser queries. */
export interface GetOrdersByUserRow {
	id: number;
	total: string;
	notes: string | null;
	createdAt: Date;
}

/** Fetch all GetOrdersByUserRow rows. */
export async function getOrdersByUser(
	client: PoolClient,
	userId: number,
): Promise<GetOrdersByUserRow[]> {
	const { rows } = await client.query<Record<string, unknown>>(
		`SELECT id, total, notes, created_at FROM orders WHERE user_id = $1 ORDER BY created_at DESC`,
		[userId],
	);
	return rows.map((row) => ({
		id: row.id as number,
		total: row.total as string,
		notes: row.notes as string | null,
		createdAt: row.created_at as Date,
	}));
}

/** Row type for GetOrderTotal queries. */
export interface GetOrderTotalRow {
	totalSum: string | null;
}

/** Fetch a single GetOrderTotalRow. */
export async function getOrderTotal(
	client: PoolClient,
	userId: number,
): Promise<GetOrderTotalRow> {
	const { rows } = await client.query<Record<string, unknown>>(
		`SELECT SUM(total) AS total_sum FROM orders WHERE user_id = $1`,
		[userId],
	);
	const row = rows[0];
	if (!row) {
		throw new Error("no row found for query: GetOrderTotal");
	}
	return {
		totalSum: row.total_sum as string | null,
	};
}

/** Row type for GetOrderWeightTotal queries. */
export interface GetOrderWeightTotalRow {
	weightTotal: number | null;
}

/** Fetch a single GetOrderWeightTotalRow. */
export async function getOrderWeightTotal(
	client: PoolClient,
	userId: number,
): Promise<GetOrderWeightTotalRow> {
	const { rows } = await client.query<Record<string, unknown>>(
		`SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = $1`,
		[userId],
	);
	const row = rows[0];
	if (!row) {
		throw new Error("no row found for query: GetOrderWeightTotal");
	}
	return {
		weightTotal: row.weight_total as number | null,
	};
}

/** Execute a query and return the number of affected rows. */
export async function deleteOrdersByUser(
	client: PoolClient,
	userId: number,
): Promise<number> {
	const result = await client.query(
		`DELETE FROM orders WHERE user_id = $1`,
		[userId],
	);
	return result.rowCount ?? 0;
}

/** Row type for GetUserById queries. */
export interface GetUserByIdRow {
	id: number;
	name: string;
	email: string | null;
	status: UserStatus;
	createdAt: Date;
}

/** Fetch a single GetUserByIdRow. */
export async function getUserById(
	client: PoolClient,
	id: number,
): Promise<GetUserByIdRow> {
	const { rows } = await client.query<Record<string, unknown>>(
		`SELECT id, name, email, status, created_at FROM users WHERE id = $1`,
		[id],
	);
	const row = rows[0];
	if (!row) {
		throw new Error("no row found for query: GetUserById");
	}
	return {
		id: row.id as number,
		name: row.name as string,
		email: row.email as string | null,
		status: row.status as UserStatus,
		createdAt: row.created_at as Date,
	};
}

/** Row type for ListActiveUsers queries. */
export interface ListActiveUsersRow {
	id: number;
	name: string;
	email: string | null;
}

/** Fetch all ListActiveUsersRow rows. */
export async function listActiveUsers(
	client: PoolClient,
	status: UserStatus,
): Promise<ListActiveUsersRow[]> {
	const { rows } = await client.query<Record<string, unknown>>(
		`SELECT id, name, email FROM users WHERE status = $1`,
		[status],
	);
	return rows.map((row) => ({
		id: row.id as number,
		name: row.name as string,
		email: row.email as string | null,
	}));
}

/** Row type for CreateUser queries. */
export interface CreateUserRow {
	id: number;
	name: string;
	email: string | null;
	status: UserStatus;
	createdAt: Date;
}

/** Fetch a single CreateUserRow. */
export async function createUser(
	client: PoolClient,
	name: string,
	email: string | null,
	status: UserStatus,
): Promise<CreateUserRow> {
	const { rows } = await client.query<Record<string, unknown>>(
		`INSERT INTO users (name, email, status) VALUES ($1, $2, $3) RETURNING id, name, email, status, created_at`,
		[name, email, status],
	);
	const row = rows[0];
	if (!row) {
		throw new Error("no row found for query: CreateUser");
	}
	return {
		id: row.id as number,
		name: row.name as string,
		email: row.email as string | null,
		status: row.status as UserStatus,
		createdAt: row.created_at as Date,
	};
}

/** Execute a query returning no rows. */
export async function updateUserEmail(
	client: PoolClient,
	email: string,
	id: number,
): Promise<void> {
	await client.query(
		`UPDATE users SET email = $1 WHERE id = $2`,
		[email, id],
	);
}

/** Execute a query returning no rows. */
export async function deleteUser(
	client: PoolClient,
	id: number,
): Promise<void> {
	await client.query(`DELETE FROM users WHERE id = $1`, [id]);
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
	client: PoolClient,
	status: UserStatus,
): Promise<GetUserOrdersRow[]> {
	const { rows } = await client.query<Record<string, unknown>>(
		`SELECT u.id, u.name, o.total, o.notes
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1`,
		[status],
	);
	return rows.map((row) => ({
		id: row.id as number,
		name: row.name as string,
		total: row.total as string | null,
		notes: row.notes as string | null,
	}));
}

/** Row type for CountUsersByStatus queries. */
export interface CountUsersByStatusRow {
	status: UserStatus;
	userCount: number;
}

/** Fetch a single CountUsersByStatusRow. */
export async function countUsersByStatus(
	client: PoolClient,
	status: UserStatus,
): Promise<CountUsersByStatusRow> {
	const { rows } = await client.query<Record<string, unknown>>(
		`SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = $1`,
		[status],
	);
	const row = rows[0];
	if (!row) {
		throw new Error("no row found for query: CountUsersByStatus");
	}
	return {
		status: row.status as UserStatus,
		userCount: row.user_count as number,
	};
}

/** Row type for GetUserWithTags queries. */
export interface GetUserWithTagsRow {
	id: number;
	name: string;
	tagName: string;
}

/** Fetch all GetUserWithTagsRow rows. */
export async function getUserWithTags(
	client: PoolClient,
	id: number,
): Promise<GetUserWithTagsRow[]> {
	const { rows } = await client.query<Record<string, unknown>>(
		`SELECT u.id, u.name, t.name AS tag_name
FROM users u
INNER JOIN user_tags ut ON u.id = ut.user_id
INNER JOIN tags t ON ut.tag_id = t.id
WHERE u.id = $1`,
		[id],
	);
	return rows.map((row) => ({
		id: row.id as number,
		name: row.name as string,
		tagName: row.tag_name as string,
	}));
}

/** Row type for SearchUsers queries. */
export interface SearchUsersRow {
	id: number;
	name: string;
	email: string | null;
}

/** Fetch all SearchUsersRow rows. */
export async function searchUsers(
	client: PoolClient,
	name: string,
): Promise<SearchUsersRow[]> {
	const { rows } = await client.query<Record<string, unknown>>(
		`SELECT id, name, email FROM users WHERE name LIKE $1`,
		[name],
	);
	return rows.map((row) => ({
		id: row.id as number,
		name: row.name as string,
		email: row.email as string | null,
	}));
}

/** Composite type user_address. */
export interface UserAddress {
	street: string;
	city: string;
	zip: string;
}

// ~keep board #204: pg has no adapter for a user-defined composite -- it hands back
// the driver's raw text form as a plain string. Parse it here instead.
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

function encodeUserAddress(value: UserAddress | null): string | null {
	if (value === null) return null;
	const encode = (field: unknown): string => {
		const text = String(field);
		if (text === "" || /[(),\"\\\s]/.test(text)) {
			return `"${text.replaceAll("\\", "\\\\").replaceAll('\"', '\"\"')}"`;
		}
		return text;
	};
	return `(${encode(value.street)},${encode(value.city)},${encode(value.zip)})`;
}

/** Row type for GetUserProfile queries. */
export interface GetUserProfileRow {
	id: number;
	secondaryStatus: UserStatus | null;
	address: UserAddress | null;
}

/** Fetch a single GetUserProfileRow. */
export async function getUserProfile(
	client: PoolClient,
	id: number,
): Promise<GetUserProfileRow> {
	const { rows } = await client.query<Record<string, unknown>>(
		`SELECT id, secondary_status, address FROM users WHERE id = $1`,
		[id],
	);
	const row = rows[0];
	if (!row) {
		throw new Error("no row found for query: GetUserProfile");
	}
	return {
		id: row.id as number,
		secondaryStatus: row.secondary_status as UserStatus | null,
		address: parseUserAddress(row.address) as UserAddress | null,
	};
}

/** Row type for RoundTripUserAddress queries. */
export interface RoundTripUserAddressRow {
	address: UserAddress | null;
}

/** Fetch a single RoundTripUserAddressRow. */
export async function roundTripUserAddress(
	client: PoolClient,
	address: UserAddress | null,
): Promise<RoundTripUserAddressRow> {
	const { rows } = await client.query<Record<string, unknown>>(
		`INSERT INTO users (name, status, address)
VALUES ('Composite Parameter Round Trip', 'active', ($1))
RETURNING address`,
		[encodeUserAddress(address)],
	);
	const row = rows[0];
	if (!row) {
		throw new Error("no row found for query: RoundTripUserAddress");
	}
	return {
		address: parseUserAddress(row.address) as UserAddress | null,
	};
}

/** Row type for GetUserAsJson queries. */
export interface GetUserAsJsonRow {
	payload: Record<string, unknown> | null;
}

/** Fetch a single GetUserAsJsonRow. */
export async function getUserAsJson(
	client: PoolClient,
	id: number,
): Promise<GetUserAsJsonRow> {
	const { rows } = await client.query<Record<string, unknown>>(
		`SELECT row_to_json(u.*) AS payload FROM users u WHERE u.id = $1`,
		[id],
	);
	const row = rows[0];
	if (!row) {
		throw new Error("no row found for query: GetUserAsJson");
	}
	return {
		payload: row.payload as Record<string, unknown> | null,
	};
}

/** Row type for GetUsersAsJson queries. */
export interface GetUsersAsJsonRow {
	payload: Record<string, unknown>[] | null;
}

/** Fetch a single GetUsersAsJsonRow. */
export async function getUsersAsJson(
	client: PoolClient,
): Promise<GetUsersAsJsonRow> {
	const { rows } = await client.query<Record<string, unknown>>(
		`SELECT jsonb_agg(u.* ORDER BY u.id) AS payload FROM users u`,
	);
	const row = rows[0];
	if (!row) {
		throw new Error("no row found for query: GetUsersAsJson");
	}
	return {
		payload: row.payload as Record<string, unknown>[] | null,
	};
}

/** Row type for GetUserOrdersAsJson queries. */
export interface GetUserOrdersAsJsonRow {
	payload: Record<string, unknown>[] | null;
}

/** Fetch a single GetUserOrdersAsJsonRow. */
export async function getUserOrdersAsJson(
	client: PoolClient,
	id: number,
): Promise<GetUserOrdersAsJsonRow> {
	const { rows } = await client.query<Record<string, unknown>>(
		`SELECT json_agg(o.* ORDER BY o.id) AS payload
FROM users u
LEFT JOIN orders o ON o.user_id = u.id
WHERE u.id = $1
GROUP BY u.id`,
		[id],
	);
	const row = rows[0];
	if (!row) {
		throw new Error("no row found for query: GetUserOrdersAsJson");
	}
	return {
		payload: row.payload as Record<string, unknown>[] | null,
	};
}
