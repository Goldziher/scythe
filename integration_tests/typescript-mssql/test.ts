import sql from "mssql";
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
	process.env["MSSQL_URL"] ??
	"sqlserver://sa:Scythe_Test1@localhost:1433?database=scythe_test";

let exitCode = 0;

function assert(condition: boolean, testName: string, detail: string): void {
	if (!condition) {
		console.error(`FAIL: ${testName}: ${detail}`);
		exitCode = 1;
	}
}

async function main(): Promise<void> {
	const url = new URL(DATABASE_URL);
	const config = {
		server: url.hostname,
		port: parseInt(url.port) || 1433,
		user: url.username,
		password: decodeURIComponent(url.password),
		database: url.searchParams.get("database") || "master",
		options: { encrypt: false, trustServerCertificate: true },
	};
	const pool = await sql.connect(config);
	try {
		// Clean slate
		for (const table of ["user_tags", "tags", "orders", "users"]) {
			await pool
				.request()
				.query(`IF OBJECT_ID('${table}', 'U') IS NOT NULL DROP TABLE ${table}`);
		}

		const { readFile } = await import("node:fs/promises");
		const schemaPath = new URL("../sql/mssql/schema.sql", import.meta.url)
			.pathname;
		const schemaSql = await readFile(schemaPath, "utf8");
		for (const stmt of schemaSql
			.split(";")
			.map((s) => s.trim())
			.filter(Boolean)) {
			await pool.request().query(stmt);
		}

		// Test: CreateUser
		const user = await createUser(pool, "Alice", "alice@example.com", true);
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
		const fetched = await getUserById(pool, userId);
		assert(fetched !== null, "GetUserById", "user should not be null");
		assert(fetched!.id === userId, "GetUserById", `expected id ${userId}`);
		assert(fetched!.name === "Alice", "GetUserById", `expected name Alice`);
		console.log("PASS: GetUserById");

		// Test: ListActiveUsers
		const activeUsers = await listActiveUsers(pool);
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
		const order = await createOrder(pool, userId, "99.95", "first order");
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
		const orders = await getOrdersByUser(pool, userId);
		assert(
			orders.length === 1,
			"GetOrdersByUser",
			`expected 1 order, got ${orders.length}`,
		);
		assert(
			orders[0]!.total === "99.95",
			"GetOrdersByUser",
			`expected total 99.95`,
		);
		console.log("PASS: GetOrdersByUser");

		// Test: DeleteUser
		const deletedOrders = await deleteOrdersByUser(pool, userId);
		assert(
			deletedOrders === 1,
			"DeleteUser",
			`expected 1 deleted order, got ${deletedOrders}`,
		);
		await deleteUser(pool, userId);
		const gone = await getUserById(pool, userId);
		assert(gone === null, "DeleteUser", "user should be null after deletion");
		console.log("PASS: DeleteUser");

		if (exitCode === 0) {
			console.log("ALL TESTS PASSED");
		}
	} finally {
		await pool.close();
	}

	process.exit(exitCode);
}

main().catch((error) => {
	console.error("Fatal error:", error);
	process.exit(1);
});
