import { Kysely, PostgresDialect, sql } from "kysely";
import pg from "pg";
import {
	createUser,
	getUserById,
	listActiveUsers,
	createOrder,
	getOrdersByUser,
	getOrderTotal,
	updateUserEmail,
	getUserOrders,
	countUsersByStatus,
	getUserWithTags,
	searchUsers,
	deleteOrdersByUser,
	deleteUser,
	UserStatusValues,
} from "./generated/queries.js";

const DATABASE_URL =
	process.env["DATABASE_URL"] ??
	"postgres://scythe:scythe@localhost:5432/scythe_test";

const db = new Kysely<any>({
	dialect: new PostgresDialect({ pool: new pg.Pool({ connectionString: DATABASE_URL }) }),
});

let exitCode = 0;

function assert(condition: boolean, testName: string, detail: string): void {
	if (!condition) {
		console.error(`FAIL: ${testName}: ${detail}`);
		exitCode = 1;
	}
}


async function main(): Promise<void> {
	try {
		// Clean slate
		await sql`DROP TABLE IF EXISTS user_tags CASCADE`.execute(db);
		await sql`DROP TABLE IF EXISTS tags CASCADE`.execute(db);
		await sql`DROP TABLE IF EXISTS orders CASCADE`.execute(db);
		await sql`DROP TABLE IF EXISTS users CASCADE`.execute(db);
		await sql`DROP TYPE IF EXISTS user_status CASCADE`.execute(db);

		await sql`CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned')`.execute(db);
		await sql`CREATE TABLE users (
      id SERIAL PRIMARY KEY,
      name TEXT NOT NULL,
      email TEXT,
      status user_status NOT NULL DEFAULT 'active',
      created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )`.execute(db);
		await sql`CREATE TABLE orders (
      id SERIAL PRIMARY KEY,
      user_id INT NOT NULL REFERENCES users (id),
      total NUMERIC(10, 2) NOT NULL,
      notes TEXT,
      created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )`.execute(db);
		await sql`CREATE TABLE tags (
      id SERIAL PRIMARY KEY,
      name TEXT NOT NULL UNIQUE
    )`.execute(db);
		await sql`CREATE TABLE user_tags (
      user_id INT NOT NULL REFERENCES users (id),
      tag_id INT NOT NULL REFERENCES tags (id),
      PRIMARY KEY (user_id, tag_id)
    )`.execute(db);

		// Test: CreateUser
		const user = await createUser(
			db,
			"Alice",
			"alice@example.com",
			UserStatusValues.Active,
		);
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
		const fetched = await getUserById(db, userId);
		assert(fetched !== null, "GetUserById", "user should not be null");
		assert(fetched!.id === userId, "GetUserById", `expected id ${userId}`);
		assert(fetched!.name === "Alice", "GetUserById", `expected name Alice`);
		console.log("PASS: GetUserById");

		// Test: ListActiveUsers
		const activeUsers = await listActiveUsers(db, UserStatusValues.Active);
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
		const order = await createOrder(db, userId, "99.95", "first order");
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
		console.log("PASS: CreateOrder");

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
		console.log("PASS: GetOrdersByUser");

		// Test: GetOrderTotal
		const orderTotal = await getOrderTotal(db, userId);
		assert(orderTotal !== null, "GetOrderTotal", "total should not be null");
		assert(
			String(orderTotal!.total_sum) === "99.95",
			"GetOrderTotal",
			`expected total_sum 99.95, got ${orderTotal!.total_sum}`,
		);
		console.log("PASS: GetOrderTotal");

		// Test: UpdateUserEmail
		await updateUserEmail(db, "alice2@example.com", userId);
		const updated = await getUserById(db, userId);
		assert(
			updated!.email === "alice2@example.com",
			"UpdateUserEmail",
			`expected updated email, got ${updated!.email}`,
		);
		console.log("PASS: UpdateUserEmail");

		// Test: GetUserOrders (LEFT JOIN)
		const bob = await createUser(db, "Bob", "bob@example.com", UserStatusValues.Active);
		const bobId = bob!.id;
		const userOrders = await getUserOrders(db, UserStatusValues.Active);
		const aliceOrderRow = userOrders.find((row) => row.id === userId);
		const bobOrderRow = userOrders.find((row) => row.id === bobId);
		assert(aliceOrderRow !== undefined, "GetUserOrders", "expected a row for Alice");
		assert(bobOrderRow !== undefined, "GetUserOrders", "expected a row for Bob");
		assert(
			aliceOrderRow!.total !== null,
			"GetUserOrders",
			"Alice has an order, total must not be null",
		);
		assert(
			bobOrderRow!.total === null && bobOrderRow!.notes === null,
			"GetUserOrders",
			"Bob has no orders, total and notes must both be null",
		);
		console.log("PASS: GetUserOrders");

		// Test: CountUsersByStatus
		const statusCount = await countUsersByStatus(db, UserStatusValues.Active);
		assert(statusCount !== null, "CountUsersByStatus", "result should not be null");
		assert(
			statusCount!.user_count >= 2,
			"CountUsersByStatus",
			`expected at least 2 active users, got ${statusCount!.user_count}`,
		);
		console.log("PASS: CountUsersByStatus");

		// Test: GetUserWithTags
		const tag = await sql<{ id: number }>`INSERT INTO tags (name) VALUES ('vip') RETURNING id`.execute(db);
		const tagId = tag.rows[0]!.id;
		await sql`INSERT INTO user_tags (user_id, tag_id) VALUES (${userId}, ${tagId})`.execute(db);
		const userTags = await getUserWithTags(db, userId);
		assert(
			userTags.length === 1,
			"GetUserWithTags",
			`expected 1 tag row, got ${userTags.length}`,
		);
		assert(
			userTags[0]!.tag_name === "vip",
			"GetUserWithTags",
			`expected tag_name vip, got ${userTags[0]!.tag_name}`,
		);
		console.log("PASS: GetUserWithTags");

		// Test: SearchUsers
		const searchResults = await searchUsers(db, "Ali%");
		assert(
			searchResults.some((row) => row.name === "Alice"),
			"SearchUsers",
			"expected Alice among search results",
		);
		console.log("PASS: SearchUsers");

		await sql`DELETE FROM user_tags WHERE user_id = ${userId}`.execute(db);
		await deleteUser(db, bobId);

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
		console.log("PASS: DeleteUser");

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
