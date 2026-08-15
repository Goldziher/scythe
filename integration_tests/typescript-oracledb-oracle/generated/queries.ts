// scythe:provenance v=0.16.0 backend=typescript-oracledb engine=oracle schema=sch1:51c12e41405f20c2 queries=q1:9b9c257a90458ab4 options=opt1:cbf29ce484222325
import oracledb from 'oracledb';


/** Row type for CreateAttachment queries. */
export interface CreateAttachmentRow {
	id: number;
	order_id: number;
	filename: string;
}

export async function createAttachment(conn: oracledb.Connection, order_id: number, filename: string, payload: Buffer, description: string | null): Promise<CreateAttachmentRow> {
	const result = await conn.execute("INSERT INTO attachments (order_id, filename, payload, description) VALUES (:1, :2, :3, :4) RETURNING id, order_id, filename INTO :5, :6, :7", [order_id, filename, payload, description, { dir: oracledb.BIND_OUT, type: oracledb.NUMBER }, { dir: oracledb.BIND_OUT, type: oracledb.NUMBER }, { dir: oracledb.BIND_OUT, type: oracledb.STRING }]);
	if (!result.outBinds) {
		throw new Error("no row found for query: CreateAttachment");
	}
	const outBinds = result.outBinds as unknown[][];
	return {
		id: (outBinds[0] ?? [])[0] as number,
		order_id: (outBinds[1] ?? [])[0] as number,
		filename: (outBinds[2] ?? [])[0] as string,
	};
}

/** Row type for GetAttachmentsByOrder queries. */
export interface GetAttachmentsByOrderRow {
	id: number;
	order_id: number;
	filename: string;
	payload: Buffer;
	description: string | null;
}

export async function getAttachmentsByOrder(conn: oracledb.Connection, order_id: number): Promise<GetAttachmentsByOrderRow[]> {
	const result = await conn.execute("SELECT id, order_id, filename, payload, description FROM attachments WHERE order_id = :1 ORDER BY id", [order_id], { outFormat: oracledb.OUT_FORMAT_OBJECT });
	if (!result.rows) {
		return [];
	}
	return result.rows.map((rawRow) => {
		const row = rawRow as Record<string, unknown>;
		return {
			id: row["ID"] as number,
			order_id: row["ORDER_ID"] as number,
			filename: row["FILENAME"] as string,
			payload: row["PAYLOAD"] as Buffer,
			description: row["DESCRIPTION"] as string | null,
		};
	});
}

/** Row type for GetAttachmentById queries. */
export interface GetAttachmentByIdRow {
	id: number;
	order_id: number;
	filename: string;
	payload: Buffer;
	description: string | null;
}

export async function getAttachmentById(conn: oracledb.Connection, id: number): Promise<GetAttachmentByIdRow | null> {
	const result = await conn.execute("SELECT id, order_id, filename, payload, description FROM attachments WHERE id = :1", [id], { outFormat: oracledb.OUT_FORMAT_OBJECT });
	if (!result.rows || result.rows.length === 0) {
		return null;
	}
	const row = result.rows[0] as Record<string, unknown>;
	return {
		id: row["ID"] as number,
		order_id: row["ORDER_ID"] as number,
		filename: row["FILENAME"] as string,
		payload: row["PAYLOAD"] as Buffer,
		description: row["DESCRIPTION"] as string | null,
	};
}

export async function deleteAttachmentsByOrder(conn: oracledb.Connection, order_id: number): Promise<number> {
	const result = await conn.execute("DELETE FROM attachments WHERE order_id = :1", [order_id]);
	return result.rowsAffected ?? 0;
}

/** Row type for CreateOrder queries. */
export interface CreateOrderRow {
	id: number;
	user_id: number;
	total: number;
	notes: string | null;
	created_at: Date;
}

export async function createOrder(conn: oracledb.Connection, user_id: number, total: number, notes: string | null): Promise<CreateOrderRow> {
	const result = await conn.execute("INSERT INTO orders (user_id, total, notes) VALUES (:1, :2, :3) RETURNING id, user_id, total, notes, created_at INTO :4, :5, :6, :7, :8", [user_id, total, notes, { dir: oracledb.BIND_OUT, type: oracledb.NUMBER }, { dir: oracledb.BIND_OUT, type: oracledb.NUMBER }, { dir: oracledb.BIND_OUT, type: oracledb.NUMBER }, { dir: oracledb.BIND_OUT, type: oracledb.STRING }, { dir: oracledb.BIND_OUT, type: oracledb.DATE }]);
	if (!result.outBinds) {
		throw new Error("no row found for query: CreateOrder");
	}
	const outBinds = result.outBinds as unknown[][];
	return {
		id: (outBinds[0] ?? [])[0] as number,
		user_id: (outBinds[1] ?? [])[0] as number,
		total: (outBinds[2] ?? [])[0] as number,
		notes: (outBinds[3] ?? [])[0] as string | null,
		created_at: (outBinds[4] ?? [])[0] as Date,
	};
}

/** Row type for GetOrdersByUser queries. */
export interface GetOrdersByUserRow {
	id: number;
	total: number;
	notes: string | null;
	created_at: Date;
}

export async function getOrdersByUser(conn: oracledb.Connection, user_id: number): Promise<GetOrdersByUserRow[]> {
	const result = await conn.execute("SELECT id, total, notes, created_at FROM orders WHERE user_id = :1 ORDER BY created_at DESC", [user_id], { outFormat: oracledb.OUT_FORMAT_OBJECT });
	if (!result.rows) {
		return [];
	}
	return result.rows.map((rawRow) => {
		const row = rawRow as Record<string, unknown>;
		return {
			id: row["ID"] as number,
			total: row["TOTAL"] as number,
			notes: row["NOTES"] as string | null,
			created_at: row["CREATED_AT"] as Date,
		};
	});
}

/** Row type for GetOrderTotal queries. */
export interface GetOrderTotalRow {
	total_sum: number | null;
}

export async function getOrderTotal(conn: oracledb.Connection, user_id: number): Promise<GetOrderTotalRow> {
	const result = await conn.execute("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = :1", [user_id], { outFormat: oracledb.OUT_FORMAT_OBJECT });
	if (!result.rows || result.rows.length === 0) {
		throw new Error("no row found for query: GetOrderTotal");
	}
	const row = result.rows[0] as Record<string, unknown>;
	return {
		total_sum: row["TOTAL_SUM"] as number | null,
	};
}

export async function deleteOrdersByUser(conn: oracledb.Connection, user_id: number): Promise<number> {
	const result = await conn.execute("DELETE FROM orders WHERE user_id = :1", [user_id]);
	return result.rowsAffected ?? 0;
}

/** Row type for GetUserById queries. */
export interface GetUserByIdRow {
	id: number;
	name: string;
	email: string | null;
	active: number;
	created_at: Date;
}

export async function getUserById(conn: oracledb.Connection, id: number): Promise<GetUserByIdRow> {
	const result = await conn.execute("SELECT id, name, email, active, created_at FROM users WHERE id = :1", [id], { outFormat: oracledb.OUT_FORMAT_OBJECT });
	if (!result.rows || result.rows.length === 0) {
		throw new Error("no row found for query: GetUserById");
	}
	const row = result.rows[0] as Record<string, unknown>;
	return {
		id: row["ID"] as number,
		name: row["NAME"] as string,
		email: row["EMAIL"] as string | null,
		active: row["ACTIVE"] as number,
		created_at: row["CREATED_AT"] as Date,
	};
}

/** Row type for ListActiveUsers queries. */
export interface ListActiveUsersRow {
	id: number;
	name: string;
	email: string | null;
}

export async function listActiveUsers(conn: oracledb.Connection): Promise<ListActiveUsersRow[]> {
	const result = await conn.execute("SELECT id, name, email FROM users WHERE active = 1", [], { outFormat: oracledb.OUT_FORMAT_OBJECT });
	if (!result.rows) {
		return [];
	}
	return result.rows.map((rawRow) => {
		const row = rawRow as Record<string, unknown>;
		return {
			id: row["ID"] as number,
			name: row["NAME"] as string,
			email: row["EMAIL"] as string | null,
		};
	});
}

/** Row type for CreateUser queries. */
export interface CreateUserRow {
	id: number;
	name: string;
	email: string | null;
	active: number;
	created_at: Date;
}

export async function createUser(conn: oracledb.Connection, name: string, email: string | null, active: number): Promise<CreateUserRow> {
	const result = await conn.execute("INSERT INTO users (name, email, active) VALUES (:1, :2, :3) RETURNING id, name, email, active, created_at INTO :4, :5, :6, :7, :8", [name, email, active, { dir: oracledb.BIND_OUT, type: oracledb.NUMBER }, { dir: oracledb.BIND_OUT, type: oracledb.STRING }, { dir: oracledb.BIND_OUT, type: oracledb.STRING }, { dir: oracledb.BIND_OUT, type: oracledb.NUMBER }, { dir: oracledb.BIND_OUT, type: oracledb.DATE }]);
	if (!result.outBinds) {
		throw new Error("no row found for query: CreateUser");
	}
	const outBinds = result.outBinds as unknown[][];
	return {
		id: (outBinds[0] ?? [])[0] as number,
		name: (outBinds[1] ?? [])[0] as string,
		email: (outBinds[2] ?? [])[0] as string | null,
		active: (outBinds[3] ?? [])[0] as number,
		created_at: (outBinds[4] ?? [])[0] as Date,
	};
}

export async function updateUserEmail(conn: oracledb.Connection, email: string, id: number): Promise<void> {
	await conn.execute("UPDATE users SET email = :1 WHERE id = :2", [email, id]);
}

export async function deleteUser(conn: oracledb.Connection, id: number): Promise<void> {
	await conn.execute("DELETE FROM users WHERE id = :1", [id]);
}

/** Row type for SearchUsers queries. */
export interface SearchUsersRow {
	id: number;
	name: string;
	email: string | null;
}

export async function searchUsers(conn: oracledb.Connection, name: string): Promise<SearchUsersRow[]> {
	const result = await conn.execute("SELECT id, name, email FROM users WHERE name LIKE :1", [name], { outFormat: oracledb.OUT_FORMAT_OBJECT });
	if (!result.rows) {
		return [];
	}
	return result.rows.map((rawRow) => {
		const row = rawRow as Record<string, unknown>;
		return {
			id: row["ID"] as number,
			name: row["NAME"] as string,
			email: row["EMAIL"] as string | null,
		};
	});
}
