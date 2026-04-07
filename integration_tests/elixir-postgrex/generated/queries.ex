defmodule UserStatus do
  @moduledoc "Enum type for user_status."

  @type t :: String.t()
  def active(), do: "active"
  def inactive(), do: "inactive"
  def banned(), do: "banned"
  def values, do: ["active", "inactive", "banned"]
end

defmodule CreateOrderRow do
  @moduledoc "Row type for CreateOrder queries."

  @type t :: %__MODULE__{
    id: integer(),
    user_id: integer(),
    total: Decimal.t(),
    notes: String.t() | nil,
    created_at: DateTime.t()
  }
  defstruct [:id, :user_id, :total, :notes, :created_at]
end

@spec create_order(pid(), integer(), Decimal.t(), String.t()) :: {:ok, %CreateOrderRow{}} | {:error, term()}
def create_order(conn, user_id, total, notes) do
  case Postgrex.query(conn, "INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3) RETURNING id, user_id, total, notes, created_at", [user_id, total, notes]) do
    {:ok, %{rows: [row]}} ->
      [id, user_id, total, notes, created_at] = row
      {:ok, %CreateOrderRow{id: id, user_id: user_id, total: total, notes: notes, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

defmodule GetOrdersByUserRow do
  @moduledoc "Row type for GetOrdersByUser queries."

  @type t :: %__MODULE__{
    id: integer(),
    total: Decimal.t(),
    notes: String.t() | nil,
    created_at: DateTime.t()
  }
  defstruct [:id, :total, :notes, :created_at]
end

@spec get_orders_by_user(pid(), integer()) :: {:ok, [%GetOrdersByUserRow{}]} | {:error, term()}
def get_orders_by_user(conn, user_id) do
  case Postgrex.query(conn, "SELECT id, total, notes, created_at FROM orders WHERE user_id = $1 ORDER BY created_at DESC", [user_id]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, total, notes, created_at] = row
        %GetOrdersByUserRow{id: id, total: total, notes: notes, created_at: created_at}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

defmodule GetOrderTotalRow do
  @moduledoc "Row type for GetOrderTotal queries."

  @type t :: %__MODULE__{
    total_sum: Decimal.t() | nil
  }
  defstruct [:total_sum]
end

@spec get_order_total(pid(), integer()) :: {:ok, %GetOrderTotalRow{}} | {:error, term()}
def get_order_total(conn, user_id) do
  case Postgrex.query(conn, "SELECT SUM(total) AS total_sum FROM orders WHERE user_id = $1", [user_id]) do
    {:ok, %{rows: [row]}} ->
      [total_sum] = row
      {:ok, %GetOrderTotalRow{total_sum: total_sum}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec delete_orders_by_user(pid(), integer()) :: {:ok, non_neg_integer()} | {:error, term()}
def delete_orders_by_user(conn, user_id) do
  case Postgrex.query(conn, "DELETE FROM orders WHERE user_id = $1", [user_id]) do
    {:ok, %{num_rows: n}} -> {:ok, n}
    {:error, err} -> {:error, err}
  end
end

defmodule GetUserByIdRow do
  @moduledoc "Row type for GetUserById queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    status: UserStatus,
    created_at: DateTime.t()
  }
  defstruct [:id, :name, :email, :status, :created_at]
end

@spec get_user_by_id(pid(), integer()) :: {:ok, %GetUserByIdRow{}} | {:error, term()}
def get_user_by_id(conn, id) do
  case Postgrex.query(conn, "SELECT id, name, email, status, created_at FROM users WHERE id = $1", [id]) do
    {:ok, %{rows: [row]}} ->
      [id, name, email, status, created_at] = row
      {:ok, %GetUserByIdRow{id: id, name: name, email: email, status: status, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

defmodule ListActiveUsersRow do
  @moduledoc "Row type for ListActiveUsers queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil
  }
  defstruct [:id, :name, :email]
end

@spec list_active_users(pid(), UserStatus) :: {:ok, [%ListActiveUsersRow{}]} | {:error, term()}
def list_active_users(conn, status) do
  case Postgrex.query(conn, "SELECT id, name, email FROM users WHERE status = $1", [status]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %ListActiveUsersRow{id: id, name: name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

defmodule CreateUserRow do
  @moduledoc "Row type for CreateUser queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    status: UserStatus,
    created_at: DateTime.t()
  }
  defstruct [:id, :name, :email, :status, :created_at]
end

@spec create_user(pid(), String.t(), String.t(), UserStatus) :: {:ok, %CreateUserRow{}} | {:error, term()}
def create_user(conn, name, email, status) do
  case Postgrex.query(conn, "INSERT INTO users (name, email, status) VALUES ($1, $2, $3) RETURNING id, name, email, status, created_at", [name, email, status]) do
    {:ok, %{rows: [row]}} ->
      [id, name, email, status, created_at] = row
      {:ok, %CreateUserRow{id: id, name: name, email: email, status: status, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec update_user_email(pid(), String.t(), integer()) :: :ok | {:error, term()}
def update_user_email(conn, email, id) do
  case Postgrex.query(conn, "UPDATE users SET email = $1 WHERE id = $2", [email, id]) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

@spec delete_user(pid(), integer()) :: :ok | {:error, term()}
def delete_user(conn, id) do
  case Postgrex.query(conn, "DELETE FROM users WHERE id = $1", [id]) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

defmodule GetUserOrdersRow do
  @moduledoc "Row type for GetUserOrders queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    total: Decimal.t() | nil,
    notes: String.t() | nil
  }
  defstruct [:id, :name, :total, :notes]
end

@spec get_user_orders(pid(), UserStatus) :: {:ok, [%GetUserOrdersRow{}]} | {:error, term()}
def get_user_orders(conn, status) do
  case Postgrex.query(conn, "SELECT u.id, u.name, o.total, o.notes
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1", [status]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, total, notes] = row
        %GetUserOrdersRow{id: id, name: name, total: total, notes: notes}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

defmodule CountUsersByStatusRow do
  @moduledoc "Row type for CountUsersByStatus queries."

  @type t :: %__MODULE__{
    status: UserStatus,
    user_count: integer()
  }
  defstruct [:status, :user_count]
end

@spec count_users_by_status(pid(), UserStatus) :: {:ok, %CountUsersByStatusRow{}} | {:error, term()}
def count_users_by_status(conn, status) do
  case Postgrex.query(conn, "SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = $1", [status]) do
    {:ok, %{rows: [row]}} ->
      [status, user_count] = row
      {:ok, %CountUsersByStatusRow{status: status, user_count: user_count}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

defmodule GetUserWithTagsRow do
  @moduledoc "Row type for GetUserWithTags queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    tag_name: String.t()
  }
  defstruct [:id, :name, :tag_name]
end

@spec get_user_with_tags(pid(), integer()) :: {:ok, [%GetUserWithTagsRow{}]} | {:error, term()}
def get_user_with_tags(conn, id) do
  case Postgrex.query(conn, "SELECT u.id, u.name, t.name AS tag_name
FROM users u
INNER JOIN user_tags ut ON u.id = ut.user_id
INNER JOIN tags t ON ut.tag_id = t.id
WHERE u.id = $1", [id]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, tag_name] = row
        %GetUserWithTagsRow{id: id, name: name, tag_name: tag_name}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

defmodule SearchUsersRow do
  @moduledoc "Row type for SearchUsers queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil
  }
  defstruct [:id, :name, :email]
end

@spec search_users(pid(), String.t()) :: {:ok, [%SearchUsersRow{}]} | {:error, term()}
def search_users(conn, name) do
  case Postgrex.query(conn, "SELECT id, name, email FROM users WHERE name LIKE $1", [name]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %SearchUsersRow{id: id, name: name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end
