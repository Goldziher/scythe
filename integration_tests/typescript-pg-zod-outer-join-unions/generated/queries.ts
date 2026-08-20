// scythe:provenance v=0.16.1 backend=typescript-pg engine=postgresql schema=sch2:c247390d575b8f71 queries=q1:b6aca93cc722fe32 options=opt1:a33ec5812d75d376
import type { PoolClient } from "pg";
import { z } from "zod";


export const UserStatusSchema = z.enum(["active", "inactive", "banned"]);

export type UserStatus = z.infer<typeof UserStatusSchema>;

export const UserStatus = {
	Active: "active",
	Inactive: "inactive",
	Banned: "banned",
} as const;

/** Row type for CreateOrder queries. */
export const CreateOrderRowSchema = z.object({
	id: z.number(),
	user_id: z.number(),
	total: z.string(),
	notes: z.string().nullable(),
	created_at: z.date(),
});

export type CreateOrderRow = z.infer<typeof CreateOrderRowSchema>;

/** Fetch a single CreateOrderRow. */
export async function createOrder(
	client: PoolClient,
	user_id: number,
	total: string,
	notes: string | null,
): Promise<CreateOrderRow> {
	const { rows } = await client.query<CreateOrderRow>(
		`INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3) RETURNING id, user_id, total, notes, created_at`,
		[user_id, total, notes],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: CreateOrder");
	}
	return row;
}

/** Row type for GetOrdersByUser queries. */
export const GetOrdersByUserRowSchema = z.object({
	id: z.number(),
	total: z.string(),
	notes: z.string().nullable(),
	created_at: z.date(),
});

export type GetOrdersByUserRow = z.infer<typeof GetOrdersByUserRowSchema>;

/** Fetch all GetOrdersByUserRow rows. */
export async function getOrdersByUser(
	client: PoolClient,
	user_id: number,
): Promise<GetOrdersByUserRow[]> {
	const { rows } = await client.query<GetOrdersByUserRow>(
		`SELECT id, total, notes, created_at FROM orders WHERE user_id = $1 ORDER BY created_at DESC`,
		[user_id],
	);
	return rows;
}

/** Row type for GetOrderTotal queries. */
export const GetOrderTotalRowSchema = z.object({
	total_sum: z.string().nullable(),
});

export type GetOrderTotalRow = z.infer<typeof GetOrderTotalRowSchema>;

/** Fetch a single GetOrderTotalRow. */
export async function getOrderTotal(
	client: PoolClient,
	user_id: number,
): Promise<GetOrderTotalRow> {
	const { rows } = await client.query<GetOrderTotalRow>(
		`SELECT SUM(total) AS total_sum FROM orders WHERE user_id = $1`,
		[user_id],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetOrderTotal");
	}
	return row;
}

/** Row type for GetOrderWeightTotal queries. */
export const GetOrderWeightTotalRowSchema = z.object({
	weight_total: z.number().nullable(),
});

export type GetOrderWeightTotalRow = z.infer<typeof GetOrderWeightTotalRowSchema>;

/** Fetch a single GetOrderWeightTotalRow. */
export async function getOrderWeightTotal(
	client: PoolClient,
	user_id: number,
): Promise<GetOrderWeightTotalRow> {
	const { rows } = await client.query<GetOrderWeightTotalRow>(
		`SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = $1`,
		[user_id],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetOrderWeightTotal");
	}
	return row;
}

/** Execute a query and return the number of affected rows. */
export async function deleteOrdersByUser(
	client: PoolClient,
	user_id: number,
): Promise<number> {
	const result = await client.query(
		`DELETE FROM orders WHERE user_id = $1`,
		[user_id],
	);
	return result.rowCount ?? 0;
}

/** Row type for GetUserById queries. */
export const GetUserByIdRowSchema = z.object({
	id: z.number(),
	name: z.string(),
	email: z.string().nullable(),
	status: UserStatusSchema,
	created_at: z.date(),
});

export type GetUserByIdRow = z.infer<typeof GetUserByIdRowSchema>;

/** Fetch a single GetUserByIdRow. */
export async function getUserById(
	client: PoolClient,
	id: number,
): Promise<GetUserByIdRow> {
	const { rows } = await client.query<GetUserByIdRow>(
		`SELECT id, name, email, status, created_at FROM users WHERE id = $1`,
		[id],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetUserById");
	}
	return row;
}

/** Row type for ListActiveUsers queries. */
export const ListActiveUsersRowSchema = z.object({
	id: z.number(),
	name: z.string(),
	email: z.string().nullable(),
});

export type ListActiveUsersRow = z.infer<typeof ListActiveUsersRowSchema>;

/** Fetch all ListActiveUsersRow rows. */
export async function listActiveUsers(
	client: PoolClient,
	status: UserStatus,
): Promise<ListActiveUsersRow[]> {
	const { rows } = await client.query<ListActiveUsersRow>(
		`SELECT id, name, email FROM users WHERE status = $1`,
		[status],
	);
	return rows;
}

/** Row type for CreateUser queries. */
export const CreateUserRowSchema = z.object({
	id: z.number(),
	name: z.string(),
	email: z.string().nullable(),
	status: UserStatusSchema,
	created_at: z.date(),
});

export type CreateUserRow = z.infer<typeof CreateUserRowSchema>;

/** Fetch a single CreateUserRow. */
export async function createUser(
	client: PoolClient,
	name: string,
	email: string | null,
	status: UserStatus,
): Promise<CreateUserRow> {
	const { rows } = await client.query<CreateUserRow>(
		`INSERT INTO users (name, email, status) VALUES ($1, $2, $3) RETURNING id, name, email, status, created_at`,
		[name, email, status],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: CreateUser");
	}
	return row;
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
export const GetUserOrdersRowSchema = z.object({
	id: z.number(),
	name: z.string(),
}).and(z.union([z.object({ total: z.string(), notes: z.string().nullable() }), z.object({ total: z.null(), notes: z.null() })]));

export type GetUserOrdersRow = z.infer<typeof GetUserOrdersRowSchema>;

/** Fetch all GetUserOrdersRow rows. */
export async function getUserOrders(
	client: PoolClient,
	status: UserStatus,
): Promise<GetUserOrdersRow[]> {
	const { rows } = await client.query<GetUserOrdersRow>(
		`SELECT u.id, u.name, o.total, o.notes
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1`,
		[status],
	);
	return rows;
}

/** Row type for CountUsersByStatus queries. */
export const CountUsersByStatusRowSchema = z.object({
	status: UserStatusSchema,
	user_count: z.number(),
});

export type CountUsersByStatusRow = z.infer<typeof CountUsersByStatusRowSchema>;

/** Fetch a single CountUsersByStatusRow. */
export async function countUsersByStatus(
	client: PoolClient,
	status: UserStatus,
): Promise<CountUsersByStatusRow> {
	const { rows } = await client.query<CountUsersByStatusRow>(
		`SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = $1`,
		[status],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: CountUsersByStatus");
	}
	return row;
}

/** Row type for GetUserWithTags queries. */
export const GetUserWithTagsRowSchema = z.object({
	id: z.number(),
	name: z.string(),
	tag_name: z.string(),
});

export type GetUserWithTagsRow = z.infer<typeof GetUserWithTagsRowSchema>;

/** Fetch all GetUserWithTagsRow rows. */
export async function getUserWithTags(
	client: PoolClient,
	id: number,
): Promise<GetUserWithTagsRow[]> {
	const { rows } = await client.query<GetUserWithTagsRow>(
		`SELECT u.id, u.name, t.name AS tag_name
FROM users u
INNER JOIN user_tags ut ON u.id = ut.user_id
INNER JOIN tags t ON ut.tag_id = t.id
WHERE u.id = $1`,
		[id],
	);
	return rows;
}

/** Row type for SearchUsers queries. */
export const SearchUsersRowSchema = z.object({
	id: z.number(),
	name: z.string(),
	email: z.string().nullable(),
});

export type SearchUsersRow = z.infer<typeof SearchUsersRowSchema>;

/** Fetch all SearchUsersRow rows. */
export async function searchUsers(
	client: PoolClient,
	name: string,
): Promise<SearchUsersRow[]> {
	const { rows } = await client.query<SearchUsersRow>(
		`SELECT id, name, email FROM users WHERE name LIKE $1`,
		[name],
	);
	return rows;
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
	return `(encode(value.street), encode(value.city), encode(value.zip))`;
}

/** Row type for GetUserProfile queries. */
export const GetUserProfileRowSchema = z.object({
	id: z.number(),
	secondary_status: UserStatusSchema.nullable(),
	address: z.custom<UserAddress>().nullable(),
});

export type GetUserProfileRow = z.infer<typeof GetUserProfileRowSchema>;

/** Fetch a single GetUserProfileRow. */
export async function getUserProfile(
	client: PoolClient,
	id: number,
): Promise<GetUserProfileRow> {
	const { rows } = await client.query<GetUserProfileRow>(
		`SELECT id, secondary_status, address FROM users WHERE id = $1`,
		[id],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetUserProfile");
	}
	return {
		...row,
		address: parseUserAddress(row.address) as UserAddress | null,
	};
}

/** Row type for RoundTripUserAddress queries. */
export const RoundTripUserAddressRowSchema = z.object({
	address: z.custom<UserAddress>().nullable(),
});

export type RoundTripUserAddressRow = z.infer<typeof RoundTripUserAddressRowSchema>;

/** Fetch a single RoundTripUserAddressRow. */
export async function roundTripUserAddress(
	client: PoolClient,
	address: UserAddress | null,
): Promise<RoundTripUserAddressRow> {
	const { rows } = await client.query<RoundTripUserAddressRow>(
		`INSERT INTO users (name, status, address)
VALUES ('Composite Parameter Round Trip', 'active', ($1))
RETURNING address`,
		[encodeUserAddress(address)],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: RoundTripUserAddress");
	}
	return {
		...row,
		address: parseUserAddress(row.address) as UserAddress | null,
	};
}

/** Row type for GetUserAsJson queries. */
export const GetUserAsJsonRowSchema = z.object({
	payload: z.record(z.string(), z.unknown()).nullable(),
});

export type GetUserAsJsonRow = z.infer<typeof GetUserAsJsonRowSchema>;

/** Fetch a single GetUserAsJsonRow. */
export async function getUserAsJson(
	client: PoolClient,
	id: number,
): Promise<GetUserAsJsonRow> {
	const { rows } = await client.query<GetUserAsJsonRow>(
		`SELECT row_to_json(u.*) AS payload FROM users u WHERE u.id = $1`,
		[id],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetUserAsJson");
	}
	return row;
}

/** Row type for GetUsersAsJson queries. */
export const GetUsersAsJsonRowSchema = z.object({
	payload: z.array(z.record(z.string(), z.unknown())).nullable(),
});

export type GetUsersAsJsonRow = z.infer<typeof GetUsersAsJsonRowSchema>;

/** Fetch a single GetUsersAsJsonRow. */
export async function getUsersAsJson(
	client: PoolClient,
): Promise<GetUsersAsJsonRow> {
	const { rows } = await client.query<GetUsersAsJsonRow>(
		`SELECT jsonb_agg(u.* ORDER BY u.id) AS payload FROM users u`,
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetUsersAsJson");
	}
	return row;
}

/** Row type for GetUserOrdersAsJson queries. */
export const GetUserOrdersAsJsonRowSchema = z.object({
	payload: z.array(z.record(z.string(), z.unknown())).nullable(),
});

export type GetUserOrdersAsJsonRow = z.infer<typeof GetUserOrdersAsJsonRowSchema>;

/** Fetch a single GetUserOrdersAsJsonRow. */
export async function getUserOrdersAsJson(
	client: PoolClient,
	id: number,
): Promise<GetUserOrdersAsJsonRow> {
	const { rows } = await client.query<GetUserOrdersAsJsonRow>(
		`SELECT json_agg(o.* ORDER BY o.id) AS payload
FROM users u
LEFT JOIN orders o ON o.user_id = u.id
WHERE u.id = $1
GROUP BY u.id`,
		[id],
	);
	const row = rows[0];
	if (row === undefined) {
		throw new Error("no row found for query: GetUserOrdersAsJson");
	}
	return row;
}
