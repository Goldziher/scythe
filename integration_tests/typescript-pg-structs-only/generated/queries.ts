// scythe:provenance v=0.16.1 backend=typescript-pg engine=postgresql schema=sch2:c247390d575b8f71 queries=q1:b6aca93cc722fe32 options=opt1:57af1d7acc85e6c7
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



/** Row type for GetOrdersByUser queries. */
export interface GetOrdersByUserRow {
	id: number;
	total: string;
	notes: string | null;
	created_at: Date;
}



/** Row type for GetOrderTotal queries. */
export interface GetOrderTotalRow {
	total_sum: string | null;
}



/** Row type for GetOrderWeightTotal queries. */
export interface GetOrderWeightTotalRow {
	weight_total: number | null;
}





/** Row type for GetUserById queries. */
export interface GetUserByIdRow {
	id: number;
	name: string;
	email: string | null;
	status: UserStatus;
	created_at: Date;
}



/** Row type for ListActiveUsers queries. */
export interface ListActiveUsersRow {
	id: number;
	name: string;
	email: string | null;
}



/** Row type for CreateUser queries. */
export interface CreateUserRow {
	id: number;
	name: string;
	email: string | null;
	status: UserStatus;
	created_at: Date;
}







/** Row type for GetUserOrders queries. */
export interface GetUserOrdersRow {
	id: number;
	name: string;
	total: string | null;
	notes: string | null;
}



/** Row type for CountUsersByStatus queries. */
export interface CountUsersByStatusRow {
	status: UserStatus;
	user_count: number;
}



/** Row type for GetUserWithTags queries. */
export interface GetUserWithTagsRow {
	id: number;
	name: string;
	tag_name: string;
}



/** Row type for SearchUsers queries. */
export interface SearchUsersRow {
	id: number;
	name: string;
	email: string | null;
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
export interface GetUserProfileRow {
	id: number;
	secondary_status: UserStatus | null;
	address: UserAddress | null;
}



/** Row type for RoundTripUserAddress queries. */
export interface RoundTripUserAddressRow {
	address: UserAddress | null;
}



/** Row type for GetUserAsJson queries. */
export interface GetUserAsJsonRow {
	payload: Record<string, unknown> | null;
}



/** Row type for GetUsersAsJson queries. */
export interface GetUsersAsJsonRow {
	payload: Record<string, unknown>[] | null;
}



/** Row type for GetUserOrdersAsJson queries. */
export interface GetUserOrdersAsJsonRow {
	payload: Record<string, unknown>[] | null;
}


