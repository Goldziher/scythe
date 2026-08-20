<?php
// scythe:provenance v=0.17.0 backend=php-pdo engine=sqlite schema=sch2:588fb635332179bc queries=q1:f7199f36438b6396 options=opt1:cbf29ce484222325

declare(strict_types=1);

namespace App\Generated;

final class RecordNotFoundException extends \RuntimeException {}


readonly class GetOrdersByUserRow {
    public function __construct(
        public int $id,
        public float $total,
        public ?string $notes,
        public string $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            total: (float) $row['total'],
            notes: $row['notes'] !== null ? (string) $row['notes'] : null,
            created_at: (string) $row['created_at'],
        );
    }
}

readonly class GetOrderTotalRow {
    public function __construct(
        public ?float $total_sum,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            total_sum: $row['total_sum'] !== null ? (float) $row['total_sum'] : null,
        );
    }
}

readonly class GetUserByIdRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public string $status,
        public string $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
            status: (string) $row['status'],
            created_at: (string) $row['created_at'],
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
     * @param \PDO $pdo
     * @param int $user_id
     * @param float $total
     * @param ?string $notes
     * @return void
     */
    public static function createOrder(\PDO $pdo, int $user_id, float $total, ?string $notes): void {
        $stmt = $pdo->prepare('INSERT INTO orders (user_id, total, notes) VALUES (:p1, :p2, :p3)');
        $stmt->execute(["p1" => $user_id, "p2" => $total, "p3" => $notes]);
    }

    /**
     * @param \PDO $pdo
     * @param int $user_id
     * @return \Generator<int, GetOrdersByUserRow, mixed, void>
     */
    public static function getOrdersByUser(\PDO $pdo, int $user_id): \Generator {
        $stmt = $pdo->prepare('SELECT id, total, notes, created_at FROM orders WHERE user_id = :p1 ORDER BY created_at DESC');
        $stmt->execute(["p1" => $user_id]);
        while ($row = $stmt->fetch(\PDO::FETCH_ASSOC)) {
            yield GetOrdersByUserRow::fromRow($row);
        }
    }

    /**
     * @param \PDO $pdo
     * @param int $user_id
     * @return GetOrderTotalRow
     * @throws RecordNotFoundException
     */
    public static function getOrderTotal(\PDO $pdo, int $user_id): GetOrderTotalRow {
        $stmt = $pdo->prepare('SELECT SUM(total) AS total_sum FROM orders WHERE user_id = :p1');
        $stmt->execute(["p1" => $user_id]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('getOrderTotal: no row found');
        }
        return GetOrderTotalRow::fromRow($row);
    }

    /**
     * @param \PDO $pdo
     * @param int $user_id
     * @return int
     */
    public static function deleteOrdersByUser(\PDO $pdo, int $user_id): int {
        $stmt = $pdo->prepare('DELETE FROM orders WHERE user_id = :p1');
        $stmt->execute(["p1" => $user_id]);
        return $stmt->rowCount();
    }

    /**
     * @param \PDO $pdo
     * @param int $id
     * @return GetUserByIdRow
     * @throws RecordNotFoundException
     */
    public static function getUserById(\PDO $pdo, int $id): GetUserByIdRow {
        $stmt = $pdo->prepare('SELECT id, name, email, status, created_at FROM users WHERE id = :p1');
        $stmt->execute(["p1" => $id]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('getUserById: no row found');
        }
        return GetUserByIdRow::fromRow($row);
    }

    /**
     * @param \PDO $pdo
     * @param string $status
     * @return \Generator<int, ListActiveUsersRow, mixed, void>
     */
    public static function listActiveUsers(\PDO $pdo, string $status): \Generator {
        $stmt = $pdo->prepare('SELECT id, name, email FROM users WHERE status = :p1');
        $stmt->execute(["p1" => $status]);
        while ($row = $stmt->fetch(\PDO::FETCH_ASSOC)) {
            yield ListActiveUsersRow::fromRow($row);
        }
    }

    /**
     * @param \PDO $pdo
     * @param string $name
     * @param ?string $email
     * @param string $status
     * @return void
     */
    public static function createUser(\PDO $pdo, string $name, ?string $email, string $status): void {
        $stmt = $pdo->prepare('INSERT INTO users (name, email, status) VALUES (:p1, :p2, :p3)');
        $stmt->execute(["p1" => $name, "p2" => $email, "p3" => $status]);
    }

    /**
     * @param \PDO $pdo
     * @param string $email
     * @param int $id
     * @return void
     */
    public static function updateUserEmail(\PDO $pdo, string $email, int $id): void {
        $stmt = $pdo->prepare('UPDATE users SET email = :p1 WHERE id = :p2');
        $stmt->execute(["p1" => $email, "p2" => $id]);
    }

    /**
     * @param \PDO $pdo
     * @param int $id
     * @return void
     */
    public static function deleteUser(\PDO $pdo, int $id): void {
        $stmt = $pdo->prepare('DELETE FROM users WHERE id = :p1');
        $stmt->execute(["p1" => $id]);
    }

    /**
     * @param \PDO $pdo
     * @param string $name
     * @return \Generator<int, SearchUsersRow, mixed, void>
     */
    public static function searchUsers(\PDO $pdo, string $name): \Generator {
        $stmt = $pdo->prepare('SELECT id, name, email FROM users WHERE name LIKE :p1');
        $stmt->execute(["p1" => $name]);
        while ($row = $stmt->fetch(\PDO::FETCH_ASSOC)) {
            yield SearchUsersRow::fromRow($row);
        }
    }

}
