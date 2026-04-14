import snowflake from "snowflake-sdk";
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
	process.env["SNOWFLAKE_URL"] ??
	"snowflake://account:password@host/database/schema";

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
	const conn = snowflake.createConnection({
		account: parsed.hostname,
		username: parsed.username,
		password: parsed.password,
		database: parsed.pathname.split("/")[1],
		schema: parsed.pathname.split("/")[2],
	});
	try {
		// Clean slate: drop tables
		for (const table of ["user_tags", "tags", "orders", "users"]) {
			try {
				await conn.execute({ sqlText: `DROP TABLE IF EXISTS ${table}` });
			} catch (_) {
				/* ignore errors */
			}
		}

		// Load and execute schema
		const { readFile } = await import("node:fs/promises");
		const schemaPath = new URL("../sql/snowflake/schema.sql", import.meta.url)
			.pathname;
		const schemaSql = await readFile(schemaPath, "utf8");
		for (const stmt of schemaSql
			.split(";")
			.map((s) => s.trim())
			.filter(Boolean)) {
			await conn.execute({ sqlText: stmt });
		}

		// Test: CreateUser
		await createUser(conn, "Alice", "alice@example.com", true, "{}");
		const users = await listActiveUsers(conn);
		const user = users.find((u) => u.name === "Alice");
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
		await createOrder(conn, userId, "99.95", "first order");
		const orders = await getOrdersByUser(conn, userId);
		const order = orders[0];
		assert(order !== null, "CreateOrder", "order should not be null");
		assert(
			order!.user_id === userId,
			"CreateOrder",
			`expected user_id ${userId}`,
		);
		assert(
			order!.total === "99.95",
			"CreateOrder",
			`expected total 99.95, got ${order!.total}`,
		);
		assert(
			order!.notes === "first order",
			"CreateOrder",
			`expected notes 'first order'`,
		);
		console.log("PASS: CreateOrder");

		// Test: GetOrdersByUser
		const allOrders = await getOrdersByUser(conn, userId);
		assert(
			allOrders.length === 1,
			"GetOrdersByUser",
			`expected 1 order, got ${allOrders.length}`,
		);
		assert(
			allOrders[0]!.total === "99.95",
			"GetOrdersByUser",
			`expected total 99.95`,
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
		await conn.destroy();
	}

	process.exit(exitCode);
}

main().catch((error) => {
	console.error("Fatal error:", error);
	process.exit(1);
});
