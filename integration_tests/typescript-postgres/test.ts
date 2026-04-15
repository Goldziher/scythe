import postgres from "postgres";
import {
	createUser,
	getUserById,
	listActiveUsers,
	createOrder,
	getOrdersByUser,
	deleteOrdersByUser,
	deleteUser,
	UserStatus,
} from "./generated/queries.js";

const DATABASE_URL =
	process.env["DATABASE_URL"] ??
	"postgres://scythe:scythe@localhost:5432/scythe_test";

const sql = postgres(DATABASE_URL);

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
		await sql`DROP TABLE IF EXISTS user_tags CASCADE`;
		await sql`DROP TABLE IF EXISTS tags CASCADE`;
		await sql`DROP TABLE IF EXISTS orders CASCADE`;
		await sql`DROP TABLE IF EXISTS users CASCADE`;
		await sql`DROP TYPE IF EXISTS user_status CASCADE`;

		await sql`CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned')`;
		await sql`CREATE TABLE users (
      id SERIAL PRIMARY KEY,
      name TEXT NOT NULL,
      email TEXT,
      status user_status NOT NULL DEFAULT 'active',
      created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )`;
		await sql`CREATE TABLE orders (
      id SERIAL PRIMARY KEY,
      user_id INT NOT NULL REFERENCES users (id),
      total NUMERIC(10, 2) NOT NULL,
      notes TEXT,
      created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )`;
		await sql`CREATE TABLE tags (
      id SERIAL PRIMARY KEY,
      name TEXT NOT NULL UNIQUE
    )`;
		await sql`CREATE TABLE user_tags (
      user_id INT NOT NULL REFERENCES users (id),
      tag_id INT NOT NULL REFERENCES tags (id),
      PRIMARY KEY (user_id, tag_id)
    )`;

		// Test: CreateUser
		const user = await createUser(
			sql,
			"Alice",
			"alice@example.com",
			UserStatus.Active,
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
		const fetched = await getUserById(sql, userId);
		assert(fetched !== null, "GetUserById", "user should not be null");
		assert(fetched!.id === userId, "GetUserById", `expected id ${userId}`);
		assert(fetched!.name === "Alice", "GetUserById", `expected name Alice`);
		console.log("PASS: GetUserById");

		// Test: ListActiveUsers
		const activeUsers = await listActiveUsers(sql, UserStatus.Active);
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
		const order = await createOrder(sql, userId, "99.95", "first order");
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
		const orders = await getOrdersByUser(sql, userId);
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

		// Test: DeleteUser
		const deletedOrders = await deleteOrdersByUser(sql, userId);
		assert(
			deletedOrders === 1,
			"DeleteUser",
			`expected 1 deleted order, got ${deletedOrders}`,
		);
		await deleteUser(sql, userId);
		const gone = await getUserById(sql, userId);
		assert(gone === null, "DeleteUser", "user should be null after deletion");
		console.log("PASS: DeleteUser");

		if (exitCode === 0) {
			console.log("ALL TESTS PASSED");
		}
	} finally {
		await sql.end();
	}

	process.exit(exitCode);
}

main().catch((error) => {
	console.error("Fatal error:", error);
	process.exit(1);
});
