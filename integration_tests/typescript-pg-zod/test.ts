import pg from "pg";
import { z } from "zod";
import {
	createUser,
	getUserById,
	listActiveUsers,
	createOrder,
	getOrdersByUser,
	deleteOrdersByUser,
	deleteUser,
	UserStatus,
	CreateUserRowSchema,
	GetUserByIdRowSchema,
	ListActiveUsersRowSchema,
	CreateOrderRowSchema,
	GetOrdersByUserRowSchema,
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

		await client.query(
			"CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned')",
		);
		await client.query(`CREATE TABLE users (
      id SERIAL PRIMARY KEY,
      name TEXT NOT NULL,
      email TEXT,
      status user_status NOT NULL DEFAULT 'active',
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

		// Test: Zod schemas exist and are ZodType instances
		assert(
			CreateUserRowSchema instanceof z.ZodType,
			"ZodSchemas",
			"CreateUserRowSchema should be a ZodType",
		);
		assert(
			GetUserByIdRowSchema instanceof z.ZodType,
			"ZodSchemas",
			"GetUserByIdRowSchema should be a ZodType",
		);
		assert(
			ListActiveUsersRowSchema instanceof z.ZodType,
			"ZodSchemas",
			"ListActiveUsersRowSchema should be a ZodType",
		);
		assert(
			CreateOrderRowSchema instanceof z.ZodType,
			"ZodSchemas",
			"CreateOrderRowSchema should be a ZodType",
		);
		assert(
			GetOrdersByUserRowSchema instanceof z.ZodType,
			"ZodSchemas",
			"GetOrdersByUserRowSchema should be a ZodType",
		);
		console.log("PASS: Zod schemas are ZodType instances");

		// Test: CreateUser
		const user = await createUser(
			client,
			"Alice",
			"alice@example.com",
			UserStatus.Active,
		);
		assert(user !== null, "CreateUser", "user should not be null");
		// Validate through Zod schema
		const parsedUser = CreateUserRowSchema.parse(user);
		assert(
			parsedUser.name === "Alice",
			"CreateUser",
			`expected name Alice, got ${parsedUser.name}`,
		);
		assert(
			parsedUser.email === "alice@example.com",
			"CreateUser",
			`expected email alice@example.com`,
		);
		const userId = user!.id;
		console.log("PASS: CreateUser");

		// Test: GetUserById
		const fetched = await getUserById(client, userId);
		assert(fetched !== null, "GetUserById", "user should not be null");
		const parsedFetched = GetUserByIdRowSchema.parse(fetched);
		assert(parsedFetched.id === userId, "GetUserById", `expected id ${userId}`);
		assert(
			parsedFetched.name === "Alice",
			"GetUserById",
			`expected name Alice`,
		);
		console.log("PASS: GetUserById");

		// Test: ListActiveUsers
		const activeUsers = await listActiveUsers(client, UserStatus.Active);
		assert(
			activeUsers.length > 0,
			"ListActiveUsers",
			"should have at least one user",
		);
		const parsedActive = ListActiveUsersRowSchema.parse(activeUsers[0]);
		assert(
			parsedActive.name === "Alice",
			"ListActiveUsers",
			"first user should be Alice",
		);
		console.log("PASS: ListActiveUsers");

		// Test: CreateOrder
		const order = await createOrder(client, userId, "99.95", "first order");
		assert(order !== null, "CreateOrder", "order should not be null");
		const parsedOrder = CreateOrderRowSchema.parse(order);
		assert(
			parsedOrder.user_id === userId,
			"CreateOrder",
			`expected user_id ${userId}`,
		);
		assert(
			parsedOrder.total === "99.95",
			"CreateOrder",
			`expected total 99.95, got ${parsedOrder.total}`,
		);
		assert(
			parsedOrder.notes === "first order",
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
		const parsedOrders = GetOrdersByUserRowSchema.parse(orders[0]);
		assert(
			parsedOrders.total === "99.95",
			"GetOrdersByUser",
			`expected total 99.95`,
		);
		console.log("PASS: GetOrdersByUser");

		// Test: DeleteUser
		const deletedOrders = await deleteOrdersByUser(client, userId);
		assert(
			deletedOrders === 1,
			"DeleteUser",
			`expected 1 deleted order, got ${deletedOrders}`,
		);
		await deleteUser(client, userId);
		const gone = await getUserById(client, userId);
		assert(gone === null, "DeleteUser", "user should be null after deletion");
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
