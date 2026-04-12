defmodule UsersStatus do
  @moduledoc "Enum type for users_status."

  @type t :: String.t()

  @spec active() :: String.t()
  def active(), do: "active"
  @spec inactive() :: String.t()
  def inactive(), do: "inactive"
  @spec banned() :: String.t()
  def banned(), do: "banned"
  @spec values() :: [String.t()]
  def values, do: ["active", "inactive", "banned"]
end

defmodule CreateOrderRow do
  @moduledoc "Row type for CreateOrder queries."

  @type t :: %__MODULE__{
    id: integer(),
    user_id: String.t(),
    total: Decimal.t(),
    notes: String.t() | nil,
    created_at: NaiveDateTime.t()
  }
  defstruct [:id, :user_id, :total, :notes, :created_at]
end

defmodule GetOrdersByUserRow do
  @moduledoc "Row type for GetOrdersByUser queries."

  @type t :: %__MODULE__{
    id: integer(),
    total: Decimal.t(),
    notes: String.t() | nil,
    created_at: NaiveDateTime.t()
  }
  defstruct [:id, :total, :notes, :created_at]
end

defmodule GetOrderTotalRow do
  @moduledoc "Row type for GetOrderTotal queries."

  @type t :: %__MODULE__{
    total_sum: Decimal.t() | nil
  }
  defstruct [:total_sum]
end

defmodule GetUserByIdRow do
  @moduledoc "Row type for GetUserById queries."

  @type t :: %__MODULE__{
    id: String.t(),
    name: String.t(),
    email: String.t() | nil,
    status: UsersStatus.t(),
    created_at: NaiveDateTime.t()
  }
  defstruct [:id, :name, :email, :status, :created_at]
end

defmodule ListActiveUsersRow do
  @moduledoc "Row type for ListActiveUsers queries."

  @type t :: %__MODULE__{
    id: String.t(),
    name: String.t(),
    email: String.t() | nil
  }
  defstruct [:id, :name, :email]
end

defmodule CreateUserRow do
  @moduledoc "Row type for CreateUser queries."

  @type t :: %__MODULE__{
    id: String.t(),
    name: String.t(),
    email: String.t() | nil
  }
  defstruct [:id, :name, :email]
end

defmodule SearchUsersRow do
  @moduledoc "Row type for SearchUsers queries."

  @type t :: %__MODULE__{
    id: String.t(),
    name: String.t(),
    email: String.t() | nil
  }
  defstruct [:id, :name, :email]
end

defmodule Scythe.Queries do

@spec create_order(MyXQL.conn(), String.t(), Decimal.t(), String.t() | nil) :: {:ok, %CreateOrderRow{}} | {:error, term()}
def create_order(conn, user_id, total, notes) do
  case MyXQL.query(conn, "INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at", [user_id, total, notes]) do
    {:ok, %MyXQL.Result{rows: [row]}} ->
      [id, user_id, total, notes, created_at] = row
      {:ok, %CreateOrderRow{id: id, user_id: user_id, total: total, notes: notes, created_at: created_at}}
    {:ok, %MyXQL.Result{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec get_orders_by_user(MyXQL.conn(), String.t()) :: {:ok, [%GetOrdersByUserRow{}]} | {:error, term()}
def get_orders_by_user(conn, user_id) do
  case MyXQL.query(conn, "SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC", [user_id]) do
    {:ok, %MyXQL.Result{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, total, notes, created_at] = row
        %GetOrdersByUserRow{id: id, total: total, notes: notes, created_at: created_at}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec get_order_total(MyXQL.conn(), String.t()) :: {:ok, %GetOrderTotalRow{}} | {:error, term()}
def get_order_total(conn, user_id) do
  case MyXQL.query(conn, "SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?", [user_id]) do
    {:ok, %MyXQL.Result{rows: [row]}} ->
      [total_sum] = row
      {:ok, %GetOrderTotalRow{total_sum: total_sum}}
    {:ok, %MyXQL.Result{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec delete_orders_by_user(MyXQL.conn(), String.t()) :: {:ok, non_neg_integer()} | {:error, term()}
def delete_orders_by_user(conn, user_id) do
  case MyXQL.query(conn, "DELETE FROM orders WHERE user_id = ?", [user_id]) do
    {:ok, %MyXQL.Result{num_rows: n}} -> {:ok, n}
    {:error, err} -> {:error, err}
  end
end

@spec get_user_by_id(MyXQL.conn(), String.t()) :: {:ok, %GetUserByIdRow{}} | {:error, term()}
def get_user_by_id(conn, id) do
  case MyXQL.query(conn, "SELECT id, name, email, status, created_at FROM users WHERE id = ?", [id]) do
    {:ok, %MyXQL.Result{rows: [row]}} ->
      [id, name, email, status, created_at] = row
      {:ok, %GetUserByIdRow{id: id, name: name, email: email, status: status, created_at: created_at}}
    {:ok, %MyXQL.Result{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec list_active_users(MyXQL.conn(), UsersStatus) :: {:ok, [%ListActiveUsersRow{}]} | {:error, term()}
def list_active_users(conn, status) do
  case MyXQL.query(conn, "SELECT id, name, email FROM users WHERE status = ?", [status]) do
    {:ok, %MyXQL.Result{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %ListActiveUsersRow{id: id, name: name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec create_user(MyXQL.conn(), String.t(), String.t() | nil, UsersStatus) :: {:ok, %CreateUserRow{}} | {:error, term()}
def create_user(conn, name, email, status) do
  case MyXQL.query(conn, "INSERT INTO users (name, email, status) VALUES (?, ?, ?) RETURNING id, name, email", [name, email, status]) do
    {:ok, %MyXQL.Result{rows: [row]}} ->
      [id, name, email] = row
      {:ok, %CreateUserRow{id: id, name: name, email: email}}
    {:ok, %MyXQL.Result{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec update_user_email(MyXQL.conn(), String.t(), String.t()) :: :ok | {:error, term()}
def update_user_email(conn, email, id) do
  case MyXQL.query(conn, "UPDATE users SET email = ? WHERE id = ?", [email, id]) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

@spec delete_user(MyXQL.conn(), String.t()) :: :ok | {:error, term()}
def delete_user(conn, id) do
  case MyXQL.query(conn, "DELETE FROM users WHERE id = ? RETURNING id", [id]) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

@spec search_users(MyXQL.conn(), String.t()) :: {:ok, [%SearchUsersRow{}]} | {:error, term()}
def search_users(conn, name) do
  case MyXQL.query(conn, "SELECT id, name, email FROM users WHERE name LIKE ?", [name]) do
    {:ok, %MyXQL.Result{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %SearchUsersRow{id: id, name: name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

end
