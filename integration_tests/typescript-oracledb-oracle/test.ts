import oracledb from "oracledb";
import {
	createUser,
	getUserById,
	listActiveUsers,
	createOrder,
	getOrdersByUser,
	deleteOrdersByUser,
	deleteUser,
} from "./generated/queries.js";

const DATABASE_URL =
	process.env["ORACLE_URL"] ?? "oracle://scythe:scythe@localhost:1521/XEPDB1";

let exitCode = 0;

function assert(condition: boolean, testName: string, detail: string): void {
	if (!condition) {
		console.error(`FAIL: ${testName}: ${detail}`);
		exitCode = 1;
	}
}

async function main(): Promise<void> {
	const { URL } = await import("node:url");
	const parsed = new URL(DATABASE_URL);
	const connectString = `${parsed.hostname}:${parsed.port}${parsed.pathname}`;
	const conn = await oracledb.getConnection({
		user: parsed.username,
		password: parsed.password,
		connectString,
	});
	try {
		// Clean slate: drop tables and sequences, ignore errors
		for (const table of ["user_tags", "tags", "orders", "users"]) {
			try {
				await conn.execute(`DROP TABLE ${table} CASCADE CONSTRAINTS`);
			} catch (_) {
				/* ignore ORA errors */
			}
		}
		for (const seq of ["tags_seq", "orders_seq", "users_seq"]) {
			try {
				await conn.execute(`DROP SEQUENCE ${seq}`);
			} catch (_) {
				/* ignore ORA errors */
			}
		}

		// Load and execute schema (PL/SQL blocks delimited by /\n)
		const { readFile } = await import("node:fs/promises");
		const schemaPath = new URL("../sql/oracle/schema_full.sql", import.meta.url)
			.pathname;
		const schemaSql = await readFile(schemaPath, "utf8");
		for (const block of schemaSql
			.split("/\n")
			.map((s) => s.trim())
			.filter(Boolean)) {
			await conn.execute(block);
		}

		// Test: CreateUser
		const user = await createUser(conn, "Alice", "alice@example.com", 1);
		assert(user !== null, "CreateUser", "user should not be null");
		assert(
			user!.name === "Alice",
			"CreateUser",
			`expected name Alice, got ${user!.name}`,
		);
		assert(
			user!.email === "alice@example.com",
			"CreateUser",
			`expected email alice@example.com`,
		);
		const userId = user!.id;
		console.log("PASS: CreateUser");

		// Test: GetUserById
		const fetched = await getUserById(conn, userId);
		assert(fetched !== null, "GetUserById", "user should not be null");
		assert(fetched!.id === userId, "GetUserById", `expected id ${userId}`);
		assert(fetched!.name === "Alice", "GetUserById", `expected name Alice`);
		console.log("PASS: GetUserById");

		// Test: ListActiveUsers
		const activeUsers = await listActiveUsers(conn);
		assert(
			activeUsers.length > 0,
			"ListActiveUsers",
			"should have at least one user",
		);
		assert(
			activeUsers[0]!.name === "Alice",
			"ListActiveUsers",
			"first user should be Alice",
		);
		console.log("PASS: ListActiveUsers");

		// Test: CreateOrder
		const order = await createOrder(conn, userId, 9999, "first order");
		assert(order !== null, "CreateOrder", "order should not be null");
		assert(
			order!.user_id === userId,
			"CreateOrder",
			`expected user_id ${userId}`,
		);
		assert(
			order!.notes === "first order",
			"CreateOrder",
			`expected notes 'first order'`,
		);
		console.log("PASS: CreateOrder");

		// Test: GetOrdersByUser
		const orders = await getOrdersByUser(conn, userId);
		assert(
			orders.length === 1,
			"GetOrdersByUser",
			`expected 1 order, got ${orders.length}`,
		);
		console.log("PASS: GetOrdersByUser");

		// Test: DeleteUser
		const deletedOrders = await deleteOrdersByUser(conn, userId);
		assert(
			deletedOrders === 1,
			"DeleteUser",
			`expected 1 deleted order, got ${deletedOrders}`,
		);
		await deleteUser(conn, userId);
		const gone = await getUserById(conn, userId);
		assert(gone === null, "DeleteUser", "user should be null after deletion");
		console.log("PASS: DeleteUser");

		if (exitCode === 0) {
			console.log("ALL TESTS PASSED");
		}
	} finally {
		await conn.close();
	}

	process.exit(exitCode);
}

main().catch((error) => {
	console.error("Fatal error:", error);
	process.exit(1);
});
