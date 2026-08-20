import postgres from "postgres";
import {
	createUser,
	getUserById,
	listActiveUsers,
	createOrder,
	getOrdersByUser,
	deleteOrdersByUser,
	deleteUser,
	getUserProfile,
	roundTripUserAddress,
	type UserAddress,
	UserStatus,
} from "./generated/queries.js";

const DATABASE_URL =
	process.env["DATABASE_URL"] ??
	"postgres://scythe:scythe@localhost:5432/scythe_test";

const sql = postgres(DATABASE_URL);

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
		await sql`DROP TABLE IF EXISTS user_tags CASCADE`;
		await sql`DROP TABLE IF EXISTS tags CASCADE`;
		await sql`DROP TABLE IF EXISTS orders CASCADE`;
		await sql`DROP TABLE IF EXISTS users CASCADE`;
		await sql`DROP TYPE IF EXISTS user_status CASCADE`;
		await sql`DROP TYPE IF EXISTS user_address CASCADE`;

		const { readFile } = await import("node:fs/promises");
		const schemaPath = new URL("../sql/pg/schema.sql", import.meta.url).pathname;
		await sql.unsafe(await readFile(schemaPath, "utf8"), [], { prepare: false });

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
		pass("CreateUser");

		// Test: GetUserById
		const fetched = await getUserById(sql, userId);
		assert(fetched !== null, "GetUserById", "user should not be null");
		assert(fetched!.id === userId, "GetUserById", `expected id ${userId}`);
		assert(fetched!.name === "Alice", "GetUserById", `expected name Alice`);
		pass("GetUserById");

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
		pass("ListActiveUsers");

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
		pass("CreateOrder");

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
		pass("GetOrdersByUser");

		// Test: GetUserProfile (board #197/#204) -- a nullable enum and a
		// nullable composite column, each observed both present and as SQL
		// NULL, plus a composite field containing a double quote and a comma
		// to prove parseUserAddress handles record_out's doubled-quote
		// escaping (board #204) rather than truncating on it.
		const presentRow = await sql`INSERT INTO users (name, email, status, secondary_status, address)
      VALUES ('Carol', 'carol@example.com', 'active', 'inactive', ROW('1 Main St', 'Springfield', '12345'))
      RETURNING id`;
		const presentId: number = presentRow[0]!.id;
		const absentRow = await sql`INSERT INTO users (name, email, status, secondary_status, address)
      VALUES ('Dave', 'dave@example.com', 'active', NULL, NULL)
      RETURNING id`;
		const absentId: number = absentRow[0]!.id;
		const quotedRow = await sql`INSERT INTO users (name, email, status, secondary_status, address)
      VALUES ('Eve', 'eve@example.com', 'active', 'inactive', ROW('12 "Main", Apt 3', 'Berlin', '10115'))
      RETURNING id`;
		const quotedId: number = quotedRow[0]!.id;

		const profile = await getUserProfile(sql, presentId);
		assert(
			profile.secondary_status === "inactive",
			"GetUserProfile",
			`expected secondary_status inactive, got ${profile.secondary_status}`,
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

		const nullProfile = await getUserProfile(sql, absentId);
		assert(
			nullProfile.secondary_status === null,
			"GetUserProfile",
			`expected secondary_status null, got ${nullProfile.secondary_status}`,
		);
		assert(
			nullProfile.address === null,
			"GetUserProfile",
			`expected address null, got ${JSON.stringify(nullProfile.address)}`,
		);

		const quotedProfile = await getUserProfile(sql, quotedId);
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
		const roundTrippedAddress = await roundTripUserAddress(sql, compositeAddress);
		assert(
			JSON.stringify(roundTrippedAddress.address) === JSON.stringify(compositeAddress),
			"RoundTripUserAddress",
			`expected ${JSON.stringify(compositeAddress)}, got ${JSON.stringify(roundTrippedAddress.address)}`,
		);
		const roundTrippedNull = await roundTripUserAddress(sql, null);
		assert(roundTrippedNull.address === null, "RoundTripUserAddress", "expected null composite");
		pass("RoundTripUserAddress", "RoundTripUserAddress (escaped fields + null)");

		await deleteUser(sql, presentId);
		await deleteUser(sql, absentId);
		await deleteUser(sql, quotedId);

		// Test: DeleteUser
		const deletedOrders = await deleteOrdersByUser(sql, userId);
		assert(
			deletedOrders === 1,
			"DeleteUser",
			`expected 1 deleted order, got ${deletedOrders}`,
		);
		await deleteUser(sql, userId);
		// GetUserById is `:one`, which errors on a missing row rather than
		// returning null. Absence is therefore observed as a throw, and
		// `gone === null` would never be reached. The flag is what makes the
		// assertion positive: a bare try/catch would pass whether or not
		// anything was thrown.
		let goneThrew = false;
		try {
			await getUserById(sql, userId);
		} catch {
			goneThrew = true;
		}
		assert(goneThrew, "DeleteUser", "user should not be found after deletion");
		pass("DeleteUser");

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
