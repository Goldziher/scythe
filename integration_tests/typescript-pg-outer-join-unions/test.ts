import pg from "pg";
import {
	createUser,
	getUserById,
	listActiveUsers,
	createOrder,
	getOrdersByUser,
	getUserOrders,
	deleteOrdersByUser,
	deleteUser,
	UserStatusValues,
} from "./generated/queries.js";

const DATABASE_URL =
	process.env["DATABASE_URL"] ??
	"postgres://scythe:scythe@localhost:5432/scythe_test";

const pool = new pg.Pool({ connectionString: DATABASE_URL });

let exitCode = 0;

function assert(condition: boolean, testName: string, detail: string): void {
	if (!condition) {
		console.error(`FAIL: ${testName}: ${detail}`);
		exitCode = 1;
	}
}


async function main(): Promise<void> {
	const client = await pool.connect();
	try {
		// Clean slate
		await client.query("DROP TABLE IF EXISTS user_tags CASCADE");
		await client.query("DROP TABLE IF EXISTS tags CASCADE");
		await client.query("DROP TABLE IF EXISTS orders CASCADE");
		await client.query("DROP TABLE IF EXISTS users CASCADE");
		await client.query("DROP TYPE IF EXISTS user_status CASCADE");
		await client.query("DROP TYPE IF EXISTS user_address CASCADE");

		await client.query(
			"CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned')",
		);
		// board #197: a nullable composite type, used by the nullable
		// `address` column below.
		await client.query(
			"CREATE TYPE user_address AS (street TEXT, city TEXT, zip TEXT)",
		);
		await client.query(`CREATE TABLE users (
      id SERIAL PRIMARY KEY,
      name TEXT NOT NULL,
      email TEXT,
      status user_status NOT NULL DEFAULT 'active',
      secondary_status user_status,
      address user_address,
      created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )`);
		await client.query(`CREATE TABLE orders (
      id SERIAL PRIMARY KEY,
      user_id INT NOT NULL REFERENCES users (id),
      total NUMERIC(10, 2) NOT NULL,
      notes TEXT,
      created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )`);
		await client.query(`CREATE TABLE tags (
      id SERIAL PRIMARY KEY,
      name TEXT NOT NULL UNIQUE
    )`);
		await client.query(`CREATE TABLE user_tags (
      user_id INT NOT NULL REFERENCES users (id),
      tag_id INT NOT NULL REFERENCES tags (id),
      PRIMARY KEY (user_id, tag_id)
    )`);

		// Test: CreateUser
		const user = await createUser(
			client,
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
		const fetched = await getUserById(client, userId);
		assert(fetched !== null, "GetUserById", "user should not be null");
		assert(fetched!.id === userId, "GetUserById", `expected id ${userId}`);
		assert(fetched!.name === "Alice", "GetUserById", `expected name Alice`);
		console.log("PASS: GetUserById");

		// Test: ListActiveUsers
		const activeUsers = await listActiveUsers(client, UserStatusValues.Active);
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
		const order = await createOrder(client, userId, "99.95", "first order");
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
		const orders = await getOrdersByUser(client, userId);
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

		// Test: GetUserOrders (LEFT JOIN discriminated union). Bob has no
		// orders, so his row must land in the "unmatched" variant (every
		// joined column null together) rather than independent optionals —
		// this is exactly the shape that would collapse back to
		// indistinguishable all-null-fields if outer_join_unions regressed.
		const bob = await createUser(
			client,
			"Bob",
			"bob@example.com",
			UserStatusValues.Active,
		);
		const bobId = bob!.id;
		const userOrders = await getUserOrders(
			client,
			UserStatusValues.Active,
		);
		const aliceRow = userOrders.find((row) => row.id === userId);
		const bobRow = userOrders.find((row) => row.id === bobId);
		assert(aliceRow !== undefined, "GetUserOrders", "expected a row for Alice");
		assert(bobRow !== undefined, "GetUserOrders", "expected a row for Bob");
		assert(
			aliceRow!.total !== null,
			"GetUserOrders",
			"matched variant: Alice has an order, total must not be null",
		);
		assert(
			bobRow!.total === null && bobRow!.notes === null,
			"GetUserOrders",
			"unmatched variant: Bob has no orders, total and notes must both be null together",
		);
		console.log("PASS: GetUserOrders (discriminated union)");
		await deleteUser(client, bobId);

		// Test: DeleteUser
		const deletedOrders = await deleteOrdersByUser(client, userId);
		assert(
			deletedOrders === 1,
			"DeleteUser",
			`expected 1 deleted order, got ${deletedOrders}`,
		);
		await deleteUser(client, userId);
		// GetUserById is `:one`, which errors on a missing row rather than
		// returning null. Absence is therefore observed as a throw, and
		// `gone === null` would never be reached. The flag is what makes the
		// assertion positive: a bare try/catch would pass whether or not
		// anything was thrown.
		let goneThrew = false;
		try {
			await getUserById(client, userId);
		} catch {
			goneThrew = true;
		}
		assert(goneThrew, "DeleteUser", "user should not be found after deletion");
		console.log("PASS: DeleteUser");

		if (exitCode === 0) {
			console.log("ALL TESTS PASSED");
		}
	} finally {
		client.release();
		await pool.end();
	}

	process.exit(exitCode);
}

main().catch((error) => {
	console.error("Fatal error:", error);
	process.exit(1);
});
