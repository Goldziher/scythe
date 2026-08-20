<?php
// scythe:provenance v=0.17.0 backend=php-amphp engine=mysql schema=sch2:4332a9c33cb39297 queries=q1:f928696deb211f90 options=opt1:cbf29ce484222325

declare(strict_types=1);

namespace App\Generated;

final class RecordNotFoundException extends \RuntimeException {}


enum UsersStatus: string {
    case ACTIVE = "active";
    case INACTIVE = "inactive";
    case BANNED = "banned";
}

readonly class GetLastInsertOrderRow {
    public function __construct(
        public int $id,
        public int $user_id,
        public string $total,
        public ?string $notes,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            user_id: (int) $row['user_id'],
            total: (string) $row['total'],
            notes: $row['notes'] !== null ? (string) $row['notes'] : null,
            created_at: new \DateTimeImmutable($row['created_at']),
        );
    }
}

readonly class GetOrdersByUserRow {
    public function __construct(
        public int $id,
        public string $total,
        public ?string $notes,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            total: (string) $row['total'],
            notes: $row['notes'] !== null ? (string) $row['notes'] : null,
            created_at: new \DateTimeImmutable($row['created_at']),
        );
    }
}

readonly class GetOrderTotalRow {
    public function __construct(
        public ?string $total_sum,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            total_sum: $row['total_sum'] !== null ? (string) $row['total_sum'] : null,
        );
    }
}

readonly class GetUserByIdRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public UsersStatus $status,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
            status: UsersStatus::from($row['status']),
            created_at: new \DateTimeImmutable($row['created_at']),
        );
    }
}

readonly class ListActiveUsersRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
        );
    }
}

readonly class GetLastInsertUserRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public UsersStatus $status,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
            status: UsersStatus::from($row['status']),
            created_at: new \DateTimeImmutable($row['created_at']),
        );
    }
}

readonly class SearchUsersRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
        );
    }
}

final class Queries {

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $user_id
     * @param string $total
     * @param ?string $notes
     * @return void
     */
    public static function createOrder(\Amp\Sql\SqlExecutor $pool, int $user_id, string $total, ?string $notes): void {
        $result = $pool->prepare('INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)')->execute([$user_id, $total, $notes]);
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @return GetLastInsertOrderRow
     * @throws RecordNotFoundException
     */
    public static function getLastInsertOrder(\Amp\Sql\SqlExecutor $pool): GetLastInsertOrderRow {
        $result = $pool->prepare('SELECT id, user_id, total, notes, created_at FROM orders WHERE id = LAST_INSERT_ID()')->execute([]);
        foreach ($result as $row) {
            return GetLastInsertOrderRow::fromRow($row);
        }
        throw new RecordNotFoundException('getLastInsertOrder: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $user_id
     * @return \Generator<int, GetOrdersByUserRow, mixed, void>
     */
    public static function getOrdersByUser(\Amp\Sql\SqlExecutor $pool, int $user_id): \Generator {
        $result = $pool->prepare('SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC')->execute([$user_id]);
        foreach ($result as $row) {
            yield GetOrdersByUserRow::fromRow($row);
        }
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $user_id
     * @return GetOrderTotalRow
     * @throws RecordNotFoundException
     */
    public static function getOrderTotal(\Amp\Sql\SqlExecutor $pool, int $user_id): GetOrderTotalRow {
        $result = $pool->prepare('SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?')->execute([$user_id]);
        foreach ($result as $row) {
            return GetOrderTotalRow::fromRow($row);
        }
        throw new RecordNotFoundException('getOrderTotal: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $user_id
     * @return int
     */
    public static function deleteOrdersByUser(\Amp\Sql\SqlExecutor $pool, int $user_id): int {
        $result = $pool->prepare('DELETE FROM orders WHERE user_id = ?')->execute([$user_id]);
        return $result->getRowCount();
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $id
     * @return GetUserByIdRow
     * @throws RecordNotFoundException
     */
    public static function getUserById(\Amp\Sql\SqlExecutor $pool, int $id): GetUserByIdRow {
        $result = $pool->prepare('SELECT id, name, email, status, created_at FROM users WHERE id = ?')->execute([$id]);
        foreach ($result as $row) {
            return GetUserByIdRow::fromRow($row);
        }
        throw new RecordNotFoundException('getUserById: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param UsersStatus $status
     * @return \Generator<int, ListActiveUsersRow, mixed, void>
     */
    public static function listActiveUsers(\Amp\Sql\SqlExecutor $pool, UsersStatus $status): \Generator {
        $result = $pool->prepare('SELECT id, name, email FROM users WHERE status = ?')->execute([$status->value]);
        foreach ($result as $row) {
            yield ListActiveUsersRow::fromRow($row);
        }
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param string $name
     * @param ?string $email
     * @param UsersStatus $status
     * @return void
     */
    public static function createUser(\Amp\Sql\SqlExecutor $pool, string $name, ?string $email, UsersStatus $status): void {
        $result = $pool->prepare('INSERT INTO users (name, email, status) VALUES (?, ?, ?)')->execute([$name, $email, $status->value]);
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @return GetLastInsertUserRow
     * @throws RecordNotFoundException
     */
    public static function getLastInsertUser(\Amp\Sql\SqlExecutor $pool): GetLastInsertUserRow {
        $result = $pool->prepare('SELECT id, name, email, status, created_at FROM users WHERE id = LAST_INSERT_ID()')->execute([]);
        foreach ($result as $row) {
            return GetLastInsertUserRow::fromRow($row);
        }
        throw new RecordNotFoundException('getLastInsertUser: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param string $email
     * @param int $id
     * @return void
     */
    public static function updateUserEmail(\Amp\Sql\SqlExecutor $pool, string $email, int $id): void {
        $result = $pool->prepare('UPDATE users SET email = ? WHERE id = ?')->execute([$email, $id]);
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $id
     * @return void
     */
    public static function deleteUser(\Amp\Sql\SqlExecutor $pool, int $id): void {
        $result = $pool->prepare('DELETE FROM users WHERE id = ?')->execute([$id]);
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param string $name
     * @return \Generator<int, SearchUsersRow, mixed, void>
     */
    public static function searchUsers(\Amp\Sql\SqlExecutor $pool, string $name): \Generator {
        $result = $pool->prepare('SELECT id, name, email FROM users WHERE name LIKE ?')->execute([$name]);
        foreach ($result as $row) {
            yield SearchUsersRow::fromRow($row);
        }
    }

}
