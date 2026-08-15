<?php

declare(strict_types=1);

if (file_exists(__DIR__ . '/vendor/autoload.php')) {
    require_once __DIR__ . '/vendor/autoload.php';
}
require_once __DIR__ . '/generated/queries.php';

use App\Generated\Queries;
use App\Generated\RecordNotFoundException;
use App\Generated\CreateUserRow;
use App\Generated\GetUserByIdRow;
use App\Generated\ListActiveUsersRow;
use App\Generated\CreateOrderRow;
use App\Generated\GetOrdersByUserRow;

function get_database_url(): string
{
    $url = getenv('SQLITE_PATH');
    if ($url === false || $url === '') {
        fwrite(STDERR, "ERROR: SQLITE_PATH environment variable is not set\n");
        exit(1);
    }
    return $url;
}

/**
 * Splits a SQL script into statements on top-level ';' only -- unlike a
 * naive `explode(';', $sql)`, this tracks single- and double-quoted spans,
 * PostgreSQL dollar-quoted bodies, and '--' line comments (an apostrophe in
 * a comment must not open a phantom string -- board #224 follow-up) so a
 * ';' inside a string literal, a `$$ ... $$` function body, or a comment
 * does not split the statement in half. '/* ... *' + '/' block comments are
 * not handled -- no schema under integration_tests/sql/ uses them today.
 *
 * @return array<string>
 */
function split_sql_statements(string $sql): array
{
    $statements = [];
    $current = '';
    $inSingle = false;
    $inDouble = false;
    $inLineComment = false;
    $dollarTag = null;
    $length = strlen($sql);
    $i = 0;
    while ($i < $length) {
        $ch = $sql[$i];
        if ($inLineComment) {
            $current .= $ch;
            if ($ch === "\n") {
                $inLineComment = false;
            }
            $i++;
            continue;
        }
        if ($dollarTag !== null) {
            $current .= $ch;
            if ($ch === '$' && substr($sql, $i, strlen($dollarTag)) === $dollarTag) {
                $current .= substr($dollarTag, 1);
                $i += strlen($dollarTag);
                $dollarTag = null;
                continue;
            }
            $i++;
            continue;
        }
        if ($inSingle) {
            $current .= $ch;
            if ($ch === "'") {
                $inSingle = false;
            }
            $i++;
            continue;
        }
        if ($inDouble) {
            $current .= $ch;
            if ($ch === '"') {
                $inDouble = false;
            }
            $i++;
            continue;
        }
        if ($ch === "'") {
            $inSingle = true;
            $current .= $ch;
            $i++;
            continue;
        }
        if ($ch === '"') {
            $inDouble = true;
            $current .= $ch;
            $i++;
            continue;
        }
        if ($ch === '-' && ($sql[$i + 1] ?? '') === '-') {
            $inLineComment = true;
            $current .= $ch;
            $i++;
            continue;
        }
        if ($ch === '$' && preg_match('/\G\$[A-Za-z0-9_]*\$/', $sql, $matches, 0, $i) === 1) {
            $dollarTag = $matches[0];
            $current .= $dollarTag;
            $i += strlen($dollarTag);
            continue;
        }
        if ($ch === ';') {
            $statements[] = $current;
            $current = '';
            $i++;
            continue;
        }
        $current .= $ch;
        $i++;
    }
    if (trim($current) !== '') {
        $statements[] = $current;
    }
    return array_values(array_filter(array_map('trim', $statements), static fn (string $stmt): bool => $stmt !== ''));
}



function create_pdo(string $path): PDO
{
    $dsn = sprintf('sqlite:%s', $path);
    $pdo = new PDO($dsn, null, null, [
        PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
        PDO::ATTR_DEFAULT_FETCH_MODE => PDO::FETCH_ASSOC,
    ]);
    return $pdo;
}

function setup_schema(PDO $pdo): void
{
    $pdo->exec("DROP TABLE IF EXISTS user_tags");
    $pdo->exec("DROP TABLE IF EXISTS tags");
    $pdo->exec("DROP TABLE IF EXISTS orders");
    $pdo->exec("DROP TABLE IF EXISTS users");
    $schema_path = __DIR__ . '/../sql/sqlite/schema.sql';
    $schema_sql = file_get_contents($schema_path);
    if ($schema_sql === false) {
        throw new RuntimeException("Failed to read schema file: {$schema_path}");
    }
    $pdo->exec($schema_sql);
}

function assert_equal(mixed $expected, mixed $actual, string $message): void
{
    if ($expected !== $actual) {
        throw new RuntimeException(
            "Assertion failed: {$message} (expected " . var_export($expected, true)
            . ", got " . var_export($actual, true) . ")"
        );
    }
}

function assert_not_null(mixed $value, string $message): void
{
    if ($value === null) {
        throw new RuntimeException("Assertion failed: {$message} (got null)");
    }
}

function assert_true(bool $value, string $message): void
{
    if (!$value) {
        throw new RuntimeException("Assertion failed: {$message}");
    }
}

function assert_null(mixed $value, string $message): void
{
    if ($value !== null) {
        throw new RuntimeException("Assertion failed: {$message} (expected null, got " . var_export($value, true) . ")");
    }
}



function test_create_user(PDO $pdo): int
{
    Queries::createUser($pdo, "Alice", "alice@example.com", "active");
    $user = Queries::getUserById($pdo, 1);
    assert_not_null($user, "CreateUser returned null");
    assert_equal("Alice", $user->name, "CreateUser name");
    assert_equal("alice@example.com", $user->email, "CreateUser email");
    echo "PASS: CreateUser\n";
    return $user->id;
}

function test_get_user_by_id(PDO $pdo, int $user_id): void
{
    $user = Queries::getUserById($pdo, $user_id);
    assert_not_null($user, "GetUserById returned null for id={$user_id}");
    assert_equal("Alice", $user->name, "GetUserById name");
    assert_equal($user_id, $user->id, "GetUserById id");
    echo "PASS: GetUserById\n";
}

function test_list_active_users(PDO $pdo): void
{
    $users = iterator_to_array(Queries::listActiveUsers($pdo, "active"));
    assert_true(count($users) >= 1, "Expected at least 1 active user, got " . count($users));
    $names = array_map(fn($u) => $u->name, $users);
    assert_true(in_array("Alice", $names, true), "Expected 'Alice' in active users");
    echo "PASS: ListActiveUsers\n";
}

function test_create_order(PDO $pdo, int $user_id): int
{
    Queries::createOrder($pdo, $user_id, 49.99, "Test order");
    $orders = iterator_to_array(Queries::getOrdersByUser($pdo, $user_id));
    $order = !empty($orders) ? $orders[0] : null;
    assert_not_null($order, "CreateOrder returned null");
    assert_equal("Test order", $order->notes, "CreateOrder notes");
    echo "PASS: CreateOrder\n";
    return $order->id;
}

function test_get_orders_by_user(PDO $pdo, int $user_id, int $order_id): void
{
    $orders = iterator_to_array(Queries::getOrdersByUser($pdo, $user_id));
    assert_true(count($orders) >= 1, "Expected at least 1 order, got " . count($orders));
    assert_equal("Test order", $orders[0]->notes, "GetOrdersByUser notes");
    $found_ids = array_map(fn($o) => $o->id, $orders);
    assert_true(in_array($order_id, $found_ids, true), "Expected order $order_id in results, got " . implode(", ", $found_ids));
    echo "PASS: GetOrdersByUser\n";
}

function test_delete_user(PDO $pdo, int $user_id): void
{
    // Delete orders first due to FK constraint
    Queries::deleteOrdersByUser($pdo, $user_id);
    Queries::deleteUser($pdo, $user_id);
    // getUserById is `:one`, so a missing row throws RecordNotFoundException rather than
    // returning null.
    try {
        Queries::getUserById($pdo, $user_id);
        throw new RuntimeException("Expected getUserById to throw RecordNotFoundException, but it returned a row");
    } catch (RecordNotFoundException) {
        // expected: the user was deleted
    }
    echo "PASS: DeleteUser\n";
}

try {
    $database_url = get_database_url();
    $pdo = create_pdo($database_url);

    setup_schema($pdo);

    $user_id = test_create_user($pdo);
    test_get_user_by_id($pdo, $user_id);
    test_list_active_users($pdo);
    $order_id = test_create_order($pdo, $user_id);
    test_get_orders_by_user($pdo, $user_id, $order_id);
    test_delete_user($pdo, $user_id);

    echo "\nALL TESTS PASSED\n";
    exit(0);
} catch (Throwable $e) {
    fwrite(STDERR, "FAIL: " . $e->getMessage() . "\n");
    fwrite(STDERR, $e->getTraceAsString() . "\n");
    exit(1);
}
