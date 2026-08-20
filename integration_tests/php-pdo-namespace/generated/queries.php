<?php
// scythe:provenance v=0.17.0 backend=php-pdo engine=postgresql schema=sch2:c247390d575b8f71 queries=q1:b6aca93cc722fe32 options=opt1:1058eb3707db2fb2

declare(strict_types=1);

namespace App\Database\Generated;

final class RecordNotFoundException extends \RuntimeException {}

final class ScytheCompositeText {
    /**
     * ~keep Splits a PostgreSQL composite's text form ("(a,b,c)") into its raw field tokens,
     * honoring its escaping rules: an empty unquoted field is SQL NULL (returned as null); a
     * field needing quoting (containing a comma, paren, quote, backslash, or leading/trailing
     * space, or the empty string) is wrapped in double quotes; every other field is unquoted and
     * taken literally. A nested composite's own "(x,y)" text form always contains parens, so it
     * always comes back quoted here, ready for that type's own fromText to parse recursively.
     *
     * Inside a quoted field record_out writes a literal '"' as '""' and a literal '\' as '\\'.
     * Both spellings must be accepted: reading '""' as "closing quote, then a new field" both
     * truncates the value and desynchronizes every field after it. Verified against
     * PostgreSQL 16 -- ROW('he said "hi"', 'back\slash', NULL) renders as
     * ("he said ""hi""","back\\slash",).
     *
     * @return array<int, string|null>
     */
    public static function parseCompositeFields(string $text): array {
        $fields = [];
        $inner = substr($text, 1, strlen($text) - 2);
        $i = 0;
        $n = strlen($inner);
        while (true) {
            $field = '';
            $isNull = false;
            if ($i < $n && $inner[$i] === '"') {
                $i++;
                while ($i < $n) {
                    $c = $inner[$i];
                    if ($c === '\\' && $i + 1 < $n) {
                        $field .= $inner[$i + 1];
                        $i += 2;
                    } elseif ($c === '"' && $i + 1 < $n && $inner[$i + 1] === '"') {
                        $field .= '"';
                        $i += 2;
                    } elseif ($c === '"') {
                        $i++;
                        break;
                    } else {
                        $field .= $c;
                        $i++;
                    }
                }
            } else {
                $start = $i;
                while ($i < $n && $inner[$i] !== ',') {
                    $i++;
                }
                $field = substr($inner, $start, $i - $start);
                $isNull = $field === '';
            }
            $fields[] = $isNull ? null : $field;
            if ($i < $n && $inner[$i] === ',') {
                $i++;
                continue;
            }
            break;
        }
        return $fields;
    }
}


enum UserStatus: string {
    case ACTIVE = "active";
    case INACTIVE = "inactive";
    case BANNED = "banned";
}

readonly class GetUserAsJsonRowPayload {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public UserStatus $status,
        public ?UserStatus $secondary_status,
        public ?UserAddress $address,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromJson(array $value): self {
        return new self(
            id: (int) $value['id'],
            name: (string) $value['name'],
            email: $value['email'] !== null ? (string) $value['email'] : null,
            status: UserStatus::from($value['status']),
            secondary_status: $value['secondary_status'] !== null ? UserStatus::from($value['secondary_status']) : null,
            address: $value['address'] !== null ? UserAddress::fromJson($value['address']) : null,
            created_at: new \DateTimeImmutable($value['created_at']),
        );
    }
}

readonly class GetUsersAsJsonRowPayload {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public UserStatus $status,
        public ?UserStatus $secondary_status,
        public ?UserAddress $address,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromJson(array $value): self {
        return new self(
            id: (int) $value['id'],
            name: (string) $value['name'],
            email: $value['email'] !== null ? (string) $value['email'] : null,
            status: UserStatus::from($value['status']),
            secondary_status: $value['secondary_status'] !== null ? UserStatus::from($value['secondary_status']) : null,
            address: $value['address'] !== null ? UserAddress::fromJson($value['address']) : null,
            created_at: new \DateTimeImmutable($value['created_at']),
        );
    }
}

readonly class GetUserOrdersAsJsonRowPayload {
    public function __construct(
        public int $id,
        public int $user_id,
        public float $total,
        public ?float $weight_kg,
        public ?string $notes,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromJson(array $value): self {
        return new self(
            id: (int) $value['id'],
            user_id: (int) $value['user_id'],
            total: (float) $value['total'],
            weight_kg: $value['weight_kg'] !== null ? (float) $value['weight_kg'] : null,
            notes: $value['notes'] !== null ? (string) $value['notes'] : null,
            created_at: new \DateTimeImmutable($value['created_at']),
        );
    }
}

readonly class CreateOrderRow {
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

readonly class GetOrderWeightTotalRow {
    public function __construct(
        public ?float $weight_total,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            weight_total: $row['weight_total'] !== null ? (float) $row['weight_total'] : null,
        );
    }
}

readonly class GetUserByIdRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public UserStatus $status,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
            status: UserStatus::from($row['status']),
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

readonly class CreateUserRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public UserStatus $status,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
            status: UserStatus::from($row['status']),
            created_at: new \DateTimeImmutable($row['created_at']),
        );
    }
}

readonly class GetUserOrdersRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $total,
        public ?string $notes,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            total: $row['total'] !== null ? (string) $row['total'] : null,
            notes: $row['notes'] !== null ? (string) $row['notes'] : null,
        );
    }
}

readonly class CountUsersByStatusRow {
    public function __construct(
        public UserStatus $status,
        public int $user_count,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            status: UserStatus::from($row['status']),
            user_count: (int) $row['user_count'],
        );
    }
}

readonly class GetUserWithTagsRow {
    public function __construct(
        public int $id,
        public string $name,
        public string $tag_name,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            tag_name: (string) $row['tag_name'],
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

readonly class UserAddress {
    public function __construct(
        public string $street,
        public string $city,
        public string $zip,
    ) {}

    public static function fromText(?string $text): ?self {
        if ($text === null) {
            return null;
        }
        $f = ScytheCompositeText::parseCompositeFields($text);
        return new self(
            (string) $f[0],
            (string) $f[1],
            (string) $f[2],
        );
    }

    public function toPgText(): string {
        return '(' . implode(',', [self::encodeCompositeField($this->street), self::encodeCompositeField($this->city), self::encodeCompositeField($this->zip)]) . ')';
    }

    private static function encodeCompositeField(mixed $value): string {
        if ($value === null) {
            return '';
        }
        if (is_object($value) && method_exists($value, 'toPgText')) {
            $raw = $value->toPgText();
        } elseif ($value instanceof \BackedEnum) {
            $raw = (string) $value->value;
        } elseif ($value instanceof \DateTimeInterface) {
            $raw = $value->format('Y-m-d H:i:sP');
        } else {
            $raw = (string) $value;
        }
        if ($raw !== '' && strpbrk($raw, ',()"\\') === false && $raw === trim($raw)) {
            return $raw;
        }
        return '"' . str_replace(['\\', '"'], ['\\\\', '""'], $raw) . '"';
    }

    public static function fromJson(array $value): self {
        return new self(
            (string) $value['street'],
            (string) $value['city'],
            (string) $value['zip'],
        );
    }
}

readonly class GetUserProfileRow {
    public function __construct(
        public int $id,
        public ?UserStatus $secondary_status,
        public ?UserAddress $address,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            secondary_status: $row['secondary_status'] !== null ? UserStatus::from($row['secondary_status']) : null,
            address: UserAddress::fromText($row['address']),
        );
    }
}

readonly class RoundTripUserAddressRow {
    public function __construct(
        public ?UserAddress $address,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            address: UserAddress::fromText($row['address']),
        );
    }
}

readonly class GetUserAsJsonRow {
    public function __construct(
        public ?GetUserAsJsonRowPayload $payload,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            payload: $row['payload'] !== null ? GetUserAsJsonRowPayload::fromJson(json_decode($row['payload'], true, 512, \JSON_THROW_ON_ERROR)) : null,
        );
    }
}

readonly class GetUsersAsJsonRow {
    public function __construct(
        /** @var ?array<GetUsersAsJsonRowPayload> */
        public ?array $payload,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            payload: $row['payload'] !== null ? array_map(static fn (array $item): GetUsersAsJsonRowPayload => GetUsersAsJsonRowPayload::fromJson($item), json_decode($row['payload'], true, 512, \JSON_THROW_ON_ERROR)) : null,
        );
    }
}

readonly class GetUserOrdersAsJsonRow {
    public function __construct(
        /** @var ?array<?GetUserOrdersAsJsonRowPayload> */
        public ?array $payload,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            payload: $row['payload'] !== null ? array_map(static fn (?array $item): ?GetUserOrdersAsJsonRowPayload => $item === null ? null : GetUserOrdersAsJsonRowPayload::fromJson($item), json_decode($row['payload'], true, 512, \JSON_THROW_ON_ERROR)) : null,
        );
    }
}

final class Queries {

    /**
     * @param \PDO $pdo
     * @param int $user_id
     * @param string $total
     * @param ?string $notes
     * @return CreateOrderRow
     * @throws RecordNotFoundException
     */
    public static function createOrder(\PDO $pdo, int $user_id, string $total, ?string $notes): CreateOrderRow {
        $stmt = $pdo->prepare('INSERT INTO orders (user_id, total, notes) VALUES (:p1, :p2, :p3) RETURNING id, user_id, total, notes, created_at');
        $stmt->execute(["p1" => $user_id, "p2" => $total, "p3" => $notes]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('createOrder: no row found');
        }
        return CreateOrderRow::fromRow($row);
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
     * @return GetOrderWeightTotalRow
     * @throws RecordNotFoundException
     */
    public static function getOrderWeightTotal(\PDO $pdo, int $user_id): GetOrderWeightTotalRow {
        $stmt = $pdo->prepare('SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = :p1');
        $stmt->execute(["p1" => $user_id]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('getOrderWeightTotal: no row found');
        }
        return GetOrderWeightTotalRow::fromRow($row);
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
     * @param UserStatus $status
     * @return \Generator<int, ListActiveUsersRow, mixed, void>
     */
    public static function listActiveUsers(\PDO $pdo, UserStatus $status): \Generator {
        $stmt = $pdo->prepare('SELECT id, name, email FROM users WHERE status = :p1');
        $stmt->execute(["p1" => $status->value]);
        while ($row = $stmt->fetch(\PDO::FETCH_ASSOC)) {
            yield ListActiveUsersRow::fromRow($row);
        }
    }

    /**
     * @param \PDO $pdo
     * @param string $name
     * @param ?string $email
     * @param UserStatus $status
     * @return CreateUserRow
     * @throws RecordNotFoundException
     */
    public static function createUser(\PDO $pdo, string $name, ?string $email, UserStatus $status): CreateUserRow {
        $stmt = $pdo->prepare('INSERT INTO users (name, email, status) VALUES (:p1, :p2, :p3) RETURNING id, name, email, status, created_at');
        $stmt->execute(["p1" => $name, "p2" => $email, "p3" => $status->value]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('createUser: no row found');
        }
        return CreateUserRow::fromRow($row);
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
     * @param UserStatus $status
     * @return \Generator<int, GetUserOrdersRow, mixed, void>
     */
    public static function getUserOrders(\PDO $pdo, UserStatus $status): \Generator {
        $stmt = $pdo->prepare('SELECT u.id, u.name, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id WHERE u.status = :p1');
        $stmt->execute(["p1" => $status->value]);
        while ($row = $stmt->fetch(\PDO::FETCH_ASSOC)) {
            yield GetUserOrdersRow::fromRow($row);
        }
    }

    /**
     * @param \PDO $pdo
     * @param UserStatus $status
     * @return CountUsersByStatusRow
     * @throws RecordNotFoundException
     */
    public static function countUsersByStatus(\PDO $pdo, UserStatus $status): CountUsersByStatusRow {
        $stmt = $pdo->prepare('SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = :p1');
        $stmt->execute(["p1" => $status->value]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('countUsersByStatus: no row found');
        }
        return CountUsersByStatusRow::fromRow($row);
    }

    /**
     * @param \PDO $pdo
     * @param int $id
     * @return \Generator<int, GetUserWithTagsRow, mixed, void>
     */
    public static function getUserWithTags(\PDO $pdo, int $id): \Generator {
        $stmt = $pdo->prepare('SELECT u.id, u.name, t.name AS tag_name FROM users u INNER JOIN user_tags ut ON u.id = ut.user_id INNER JOIN tags t ON ut.tag_id = t.id WHERE u.id = :p1');
        $stmt->execute(["p1" => $id]);
        while ($row = $stmt->fetch(\PDO::FETCH_ASSOC)) {
            yield GetUserWithTagsRow::fromRow($row);
        }
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

    /**
     * @param \PDO $pdo
     * @param int $id
     * @return GetUserProfileRow
     * @throws RecordNotFoundException
     */
    public static function getUserProfile(\PDO $pdo, int $id): GetUserProfileRow {
        $stmt = $pdo->prepare('SELECT id, secondary_status, address FROM users WHERE id = :p1');
        $stmt->execute(["p1" => $id]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('getUserProfile: no row found');
        }
        return GetUserProfileRow::fromRow($row);
    }

    /**
     * @param \PDO $pdo
     * @param ?UserAddress $address
     * @return RoundTripUserAddressRow
     * @throws RecordNotFoundException
     */
    public static function roundTripUserAddress(\PDO $pdo, ?UserAddress $address): RoundTripUserAddressRow {
        $stmt = $pdo->prepare('INSERT INTO users (name, status, address) VALUES (\'Composite Parameter Round Trip\', \'active\', (:p1::text::user_address)) RETURNING address');
        $stmt->execute(["p1" => $address?->toPgText()]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('roundTripUserAddress: no row found');
        }
        return RoundTripUserAddressRow::fromRow($row);
    }

    /**
     * @param \PDO $pdo
     * @param int $id
     * @return GetUserAsJsonRow
     * @throws RecordNotFoundException
     */
    public static function getUserAsJson(\PDO $pdo, int $id): GetUserAsJsonRow {
        $stmt = $pdo->prepare('SELECT row_to_json(u.*) AS payload FROM users u WHERE u.id = :p1');
        $stmt->execute(["p1" => $id]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('getUserAsJson: no row found');
        }
        return GetUserAsJsonRow::fromRow($row);
    }

    /**
     * @param \PDO $pdo
     * @return GetUsersAsJsonRow
     * @throws RecordNotFoundException
     */
    public static function getUsersAsJson(\PDO $pdo): GetUsersAsJsonRow {
        $stmt = $pdo->prepare('SELECT jsonb_agg(u.* ORDER BY u.id) AS payload FROM users u');
        $stmt->execute();
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('getUsersAsJson: no row found');
        }
        return GetUsersAsJsonRow::fromRow($row);
    }

    /**
     * @param \PDO $pdo
     * @param int $id
     * @return GetUserOrdersAsJsonRow
     * @throws RecordNotFoundException
     */
    public static function getUserOrdersAsJson(\PDO $pdo, int $id): GetUserOrdersAsJsonRow {
        $stmt = $pdo->prepare('SELECT json_agg(o.* ORDER BY o.id) AS payload FROM users u LEFT JOIN orders o ON o.user_id = u.id WHERE u.id = :p1 GROUP BY u.id');
        $stmt->execute(["p1" => $id]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('getUserOrdersAsJson: no row found');
        }
        return GetUserOrdersAsJsonRow::fromRow($row);
    }

}
