import { Kysely, MysqlDialect, sql } from "kysely";
import mysql from "mysql2";
import {
	createUser,
	getUserById,
	listActiveUsers,
	createOrder,
	getOrdersByUser,
	deleteOrdersByUser,
	deleteUser,
	UsersStatusValues,
	getLastInsertUser,
	getLastInsertOrder,
} from "./generated/queries.js";

const DATABASE_URL =
	process.env["MYSQL_URL"] ??
	"mysql://root@localhost:3306/scythe_test";

const db = new Kysely<any>({
	dialect: new MysqlDialect({ pool: mysql.createPool(DATABASE_URL) }),
});

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
		await sql.raw("DROP TABLE IF EXISTS user_tags").execute(db);
		await sql.raw("DROP TABLE IF EXISTS tags").execute(db);
		await sql.raw("DROP TABLE IF EXISTS orders").execute(db);
		await sql.raw("DROP TABLE IF EXISTS users").execute(db);

		const schemaPath = new URL(
			"../sql/mysql/schema.sql",
			import.meta.url,
		).pathname;
		const { readFile } = await import("node:fs/promises");
		const schemaSql = await readFile(schemaPath, "utf8");
		for (const stmt of schemaSql.split(";").map((s) => s.trim()).filter(Boolean)) {
			await sql.raw(stmt).execute(db);
		}

		// Test: CreateUser
		await createUser(db, "Alice", "alice@example.com", UsersStatusValues.Active);
		const user = await getLastInsertUser(db);
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
		pass("CreateUser");

		// Test: GetUserById
		const fetched = await getUserById(db, userId);
		assert(fetched !== null, "GetUserById", "user should not be null");
		assert(fetched!.id === userId, "GetUserById", `expected id ${userId}`);
		assert(fetched!.name === "Alice", "GetUserById", `expected name Alice`);
		pass("GetUserById");

		// Test: ListActiveUsers
		const activeUsers = await listActiveUsers(db, UsersStatusValues.Active);
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
		await createOrder(db, userId, "99.95", "first order");
		const order = await getLastInsertOrder(db);
		assert(order !== null, "CreateOrder", "order should not be null");
		assert(
			order!.user_id === userId,
			"CreateOrder",
			`expected user_id ${userId}`,
		);
		assert(
			String(order!.total) === "99.95",
			"CreateOrder",
			`expected total 99.95, got ${order!.total}`,
		);
		assert(
			order!.notes === "first order",
			"CreateOrder",
			`expected notes 'first order'`,
		);
		pass("CreateOrder");

		// Test: GetOrdersByUser
		const orders = await getOrdersByUser(db, userId);
		assert(
			orders.length === 1,
			"GetOrdersByUser",
			`expected 1 order, got ${orders.length}`,
		);
		assert(
			String(orders[0]!.total) === "99.95",
			"GetOrdersByUser",
			`expected total 99.95`,
		);
		pass("GetOrdersByUser");

		// Test: DeleteUser
		const deletedOrders = await deleteOrdersByUser(db, userId);
		assert(
			deletedOrders === 1,
			"DeleteUser",
			`expected 1 deleted order, got ${deletedOrders}`,
		);
		await deleteUser(db, userId);
		// GetUserById is `:one`, which errors on a missing row rather than
		// returning null. Absence is therefore observed as a throw, and
		// `gone === null` would never be reached. The flag is what makes the
		// assertion positive: a bare try/catch would pass whether or not
		// anything was thrown.
		let goneThrew = false;
		try {
			await getUserById(db, userId);
		} catch {
			goneThrew = true;
		}
		assert(goneThrew, "DeleteUser", "user should not be found after deletion");
		pass("DeleteUser");

		if (exitCode === 0) {
			console.log("ALL TESTS PASSED");
		}
	} finally {
		await db.destroy();
	}

	process.exit(exitCode);
}

main().catch((error) => {
	console.error("Fatal error:", error);
	process.exit(1);
});
