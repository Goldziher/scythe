import { CamelCasePlugin, Kysely, PostgresDialect, sql } from "kysely";
import pg from "pg";
import {
	createUser,
	getUserById,
	listActiveUsers,
	createOrder,
	getOrdersByUser,
	getOrderTotal,
	updateUserEmail,
	searchUsers,
	getUserOrders,
	countUsersByStatus,
	getUserWithTags,
	getMultipleUnderscoreAlias,
	deleteOrdersByUser,
	deleteUser,
	getUserProfile,
	roundTripUserAddress,
	type UserAddress,
	UserStatusValues,
} from "./generated/queries.js";

const DATABASE_URL =
	process.env["DATABASE_URL"] ??
	"postgres://scythe:scythe@localhost:5432/scythe_test";

const db = new Kysely<any>({
	dialect: new PostgresDialect({ pool: new pg.Pool({ connectionString: DATABASE_URL }) }),
	plugins: [new CamelCasePlugin()],
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

// Splits a SQL script into individual statements on top-level `;` only --
// unlike a naive `.split(";")`, this tracks single-quoted and
// double-quoted spans, PostgreSQL dollar-quoted bodies, and `--` line
// comments (an apostrophe in a comment must not open a phantom string --
// board #224 follow-up) so a `;` inside a string literal, a `$$ ... $$`
// function body, or a comment does not split the statement in half.
// `/* ... */` block comments are not handled -- no schema under
// integration_tests/sql/ uses them today.
function splitSqlStatements(sql: string): string[] {
	const statements: string[] = [];
	let current = "";
	let inSingle = false;
	let inDouble = false;
	let inLineComment = false;
	let dollarTag: string | null = null;
	for (let i = 0; i < sql.length; i++) {
		const ch = sql[i];
		if (inLineComment) {
			current += ch;
			if (ch === "\n") inLineComment = false;
			continue;
		}
		if (dollarTag !== null) {
			current += ch;
			if (ch === "$" && sql.startsWith(dollarTag, i)) {
				current += dollarTag.slice(1);
				i += dollarTag.length - 1;
				dollarTag = null;
			}
			continue;
		}
		if (inSingle) {
			current += ch;
			if (ch === "'") inSingle = false;
			continue;
		}
		if (inDouble) {
			current += ch;
			if (ch === '"') inDouble = false;
			continue;
		}
		if (ch === "-" && sql[i + 1] === "-") {
			inLineComment = true;
			current += ch;
			continue;
		}
		if (ch === "'") {
			inSingle = true;
			current += ch;
			continue;
		}
		if (ch === '"') {
			inDouble = true;
			current += ch;
			continue;
		}
		if (ch === "$") {
			const match = /^\$[A-Za-z0-9_]*\$/.exec(sql.slice(i));
			if (match) {
				dollarTag = match[0];
				current += dollarTag;
				i += dollarTag.length - 1;
				continue;
			}
		}
		if (ch === ";") {
			statements.push(current);
			current = "";
			continue;
		}
		current += ch;
	}
	if (current.trim() !== "") statements.push(current);
	return statements.map((s) => s.trim()).filter(Boolean);
}


async function main(): Promise<void> {
	try {
		// Clean slate
		await sql`DROP TABLE IF EXISTS user_tags CASCADE`.execute(db);
		await sql`DROP TABLE IF EXISTS tags CASCADE`.execute(db);
		await sql`DROP TABLE IF EXISTS orders CASCADE`.execute(db);
		await sql`DROP TABLE IF EXISTS users CASCADE`.execute(db);
		await sql`DROP TYPE IF EXISTS user_status CASCADE`.execute(db);
		await sql`DROP TYPE IF EXISTS user_address CASCADE`.execute(db);

		const { readFile } = await import("node:fs/promises");
		const schemaPath = new URL("../sql/pg/schema.sql", import.meta.url).pathname;
		await sql.raw(await readFile(schemaPath, "utf8")).execute(db);

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
		pass("CreateUser");

		// Test: GetUserById
		const fetched = await getUserById(db, userId);
		assert(fetched !== null, "GetUserById", "user should not be null");
		assert(fetched!.id === userId, "GetUserById", `expected id ${userId}`);
		assert(fetched!.name === "Alice", "GetUserById", `expected name Alice`);
		pass("GetUserById");

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
		pass("ListActiveUsers");

		// Test: CreateOrder
		const order = await createOrder(db, userId, "99.95", "first order");
		assert(order !== null, "CreateOrder", "order should not be null");
		assert(
			order!.userId === userId,
			"CreateOrder",
			`expected userId ${userId}`,
		);
		assert(order!.id > 0, "CreateOrder", "single-word id key should remain unchanged");
		assert(!("user_id" in order!), "CreateOrder", "row should omit the snake_case user_id key");
		const pluginRow = await getMultipleUnderscoreAlias(db, userId);
		assert(
			pluginRow.multipleUnderscoreAlias === userId,
			"CreateOrder",
			"CamelCasePlugin should remap a multiple-underscore alias",
		);
		assert(
			pluginRow !== undefined && !("multiple_underscore_alias" in pluginRow),
			"CreateOrder",
			"row should omit the multiple-underscore SQL alias",
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

		// Test: GetOrderTotal
		const orderTotal = await getOrderTotal(db, userId);
		assert(orderTotal !== null, "GetOrderTotal", "total should not be null");
		assert(
			String(orderTotal!.totalSum) === "99.95",
			"GetOrderTotal",
			`expected total_sum 99.95, got ${orderTotal!.totalSum}`,
		);
		pass("GetOrderTotal");

		// Test: UpdateUserEmail
		await updateUserEmail(db, "alice2@example.com", userId);
		const updated = await getUserById(db, userId);
		assert(
			updated!.email === "alice2@example.com",
			"UpdateUserEmail",
			`expected updated email, got ${updated!.email}`,
		);
		pass("UpdateUserEmail");
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
		pass("GetUserOrders");

		// Test: CountUsersByStatus
		const statusCount = await countUsersByStatus(db, UserStatusValues.Active);
		assert(statusCount !== null, "CountUsersByStatus", "result should not be null");
		assert(
			statusCount!.userCount >= 2,
			"CountUsersByStatus",
			`expected at least 2 active users, got ${statusCount!.userCount}`,
		);
		pass("CountUsersByStatus");

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
			userTags[0]!.tagName === "vip",
			"GetUserWithTags",
			`expected tag_name vip, got ${userTags[0]!.tagName}`,
		);
		pass("GetUserWithTags");

		// Test: SearchUsers
		const searchResults = await searchUsers(db, "Ali%");
		assert(
			searchResults.some((row) => row.name === "Alice"),
			"SearchUsers",
			"expected Alice among search results",
		);
		pass("SearchUsers");
		// Test: GetUserProfile (board #197/#204) -- a nullable enum and a
		// nullable composite column, each observed both present and as SQL
		// NULL, plus a composite field containing a double quote and a comma
		// to prove parseUserAddress handles record_out's doubled-quote
		// escaping (board #204) rather than truncating on it.
		const presentRow = await sql<{ id: number }>`INSERT INTO users (name, email, status, secondary_status, address)
      VALUES ('Carol', 'carol@example.com', 'active', 'inactive', ROW('1 Main St', 'Springfield', '12345'))
      RETURNING id`.execute(db);
		const presentId = presentRow.rows[0]!.id;
		const absentRow = await sql<{ id: number }>`INSERT INTO users (name, email, status, secondary_status, address)
      VALUES ('Dave', 'dave@example.com', 'active', NULL, NULL)
      RETURNING id`.execute(db);
		const absentId = absentRow.rows[0]!.id;
		const quotedRow = await sql<{ id: number }>`INSERT INTO users (name, email, status, secondary_status, address)
      VALUES ('Eve', 'eve@example.com', 'active', 'inactive', ROW('12 "Main", Apt 3', 'Berlin', '10115'))
      RETURNING id`.execute(db);
		const quotedId = quotedRow.rows[0]!.id;

		const profile = await getUserProfile(db, presentId);
		assert(
			profile.secondaryStatus === "inactive",
			"GetUserProfile",
			`expected secondary_status inactive, got ${profile.secondaryStatus}`,
		);
		assert(profile.address !== null, "GetUserProfile", "expected address to be present");
		assert(
			profile.address!.street === "1 Main St",
			"GetUserProfile",
			`expected address.street '1 Main St', got ${profile.address!.street}`,
		);
		assert(
			profile.address!.city === "Springfield",
			"GetUserProfile",
			`expected address.city 'Springfield', got ${profile.address!.city}`,
		);
		assert(
			profile.address!.zip === "12345",
			"GetUserProfile",
			`expected address.zip '12345', got ${profile.address!.zip}`,
		);

		const nullProfile = await getUserProfile(db, absentId);
		assert(
			nullProfile.secondaryStatus === null,
			"GetUserProfile",
			`expected secondary_status null, got ${nullProfile.secondaryStatus}`,
		);
		assert(
			nullProfile.address === null,
			"GetUserProfile",
			`expected address null, got ${JSON.stringify(nullProfile.address)}`,
		);

		const quotedProfile = await getUserProfile(db, quotedId);
		assert(quotedProfile.address !== null, "GetUserProfile", "expected quoted address to be present");
		assert(
			quotedProfile.address!.street === '12 "Main", Apt 3',
			"GetUserProfile",
			`expected address.street '12 "Main", Apt 3', got ${quotedProfile.address!.street}`,
		);
		assert(
			quotedProfile.address!.city === "Berlin",
			"GetUserProfile",
			`expected address.city 'Berlin', got ${quotedProfile.address!.city}`,
		);
		assert(
			quotedProfile.address!.zip === "10115",
			"GetUserProfile",
			`expected address.zip '10115', got ${quotedProfile.address!.zip}`,
		);
		pass("GetUserProfile", "GetUserProfile (nullable enum + composite)");

		const compositeAddress: UserAddress = {
			street: '12 "Main", Apt \\3',
			city: "",
			zip: "10115",
		};
		const roundTrippedAddress = await roundTripUserAddress(db, compositeAddress);
		assert(
			JSON.stringify(roundTrippedAddress.address) === JSON.stringify(compositeAddress),
			"RoundTripUserAddress",
			`expected ${JSON.stringify(compositeAddress)}, got ${JSON.stringify(roundTrippedAddress.address)}`,
		);
		const roundTrippedNull = await roundTripUserAddress(db, null);
		assert(roundTrippedNull.address === null, "RoundTripUserAddress", "expected null composite");
		pass("RoundTripUserAddress", "RoundTripUserAddress (escaped fields + null)");

		await deleteUser(db, presentId);
		await deleteUser(db, absentId);
		await deleteUser(db, quotedId);
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
