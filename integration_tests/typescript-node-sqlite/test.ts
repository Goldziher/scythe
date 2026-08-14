import { DatabaseSync } from "node:sqlite";
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
	process.env["SQLITE_PATH"] ??
	"test.db";

const db = new DatabaseSync(DATABASE_URL);

let exitCode = 0;
const failedTests = new Set<string>();

function assert(condition: boolean, testName: string, detail: string): void {
	if (!condition) {
		console.error(`FAIL: ${testName}: ${detail}`);
		exitCode = 1;
		failedTests.add(testName);
	}
}

function pass(testName: string, label: string = testName): void {
	if (!failedTests.has(testName)) {
		console.log(`PASS: ${label}`);
	}
}


async function main(): Promise<void> {
	try {
		// Clean slate
		db.exec("DROP TABLE IF EXISTS user_tags");
		db.exec("DROP TABLE IF EXISTS tags");
		db.exec("DROP TABLE IF EXISTS orders");
		db.exec("DROP TABLE IF EXISTS users");

		const schemaPath = new URL("../sql/sqlite/schema.sql", import.meta.url).pathname;
		const { readFile } = await import("node:fs/promises");
		const schemaSql = await readFile(schemaPath, "utf8");
		db.exec(schemaSql);

		// Test: CreateUser
		createUser(db, "Alice", "alice@example.com", "active");
		const insertedUserId = db.prepare("SELECT last_insert_rowid() as id").get() as { id: number };
		const userId = insertedUserId.id;
		const user = getUserById(db, userId);
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
		pass("CreateUser");

		// Test: GetUserById
		const fetched = getUserById(db, userId);
		assert(fetched !== null, "GetUserById", "user should not be null");
		assert(fetched!.id === userId, "GetUserById", `expected id ${userId}`);
		assert(fetched!.name === "Alice", "GetUserById", `expected name Alice`);
		pass("GetUserById");

		// Test: ListActiveUsers
		const activeUsers = listActiveUsers(db, "active");
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
		pass("ListActiveUsers");

		// Test: CreateOrder
		createOrder(db, userId, 99.95, "first order");
		const insertedOrderId = db.prepare("SELECT last_insert_rowid() as id").get() as { id: number };
		const orderId = insertedOrderId.id;
		assert(orderId !== null, "CreateOrder", "order id should not be null");
		pass("CreateOrder");

		// Test: GetOrdersByUser
		const orders = getOrdersByUser(db, userId);
		assert(
			orders.length === 1,
			"GetOrdersByUser",
			`expected 1 order, got ${orders.length}`,
		);
		assert(
			Number(orders[0]!.total) === 99.95,
			"GetOrdersByUser",
			`expected total 99.95`,
		);
		pass("GetOrdersByUser");

		// Test: DeleteUser
		const deletedOrders = deleteOrdersByUser(db, userId);
		assert(
			deletedOrders === 1,
			"DeleteUser",
			`expected 1 deleted order, got ${deletedOrders}`,
		);
		deleteUser(db, userId);
		// GetUserById is `:one`, which errors on a missing row rather than
		// returning null. Absence is therefore observed as a throw, and
		// `gone === null` would never be reached. The flag is what makes the
		// assertion positive: a bare try/catch would pass whether or not
		// anything was thrown.
		let goneThrew = false;
		try {
			getUserById(db, userId);
		} catch {
			goneThrew = true;
		}
		assert(goneThrew, "DeleteUser", "user should not be found after deletion");
		pass("DeleteUser");

		if (exitCode === 0) {
			console.log("ALL TESTS PASSED");
		}
	} finally {
		db.close();
	}

	process.exit(exitCode);
}

main().catch((error) => {
	console.error("Fatal error:", error);
	process.exit(1);
});
