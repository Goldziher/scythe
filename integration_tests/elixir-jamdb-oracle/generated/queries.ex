defmodule CreateOrderRow do
  @moduledoc "Row type for CreateOrder queries."

  @type t :: %__MODULE__{
    id: integer(),
    user_id: integer(),
    total: integer(),
    notes: String.t() | nil,
    created_at: NaiveDateTime.t()
  }
  defstruct [:id, :user_id, :total, :notes, :created_at]
end

defmodule GetOrdersByUserRow do
  @moduledoc "Row type for GetOrdersByUser queries."

  @type t :: %__MODULE__{
    id: integer(),
    total: integer(),
    notes: String.t() | nil,
    created_at: NaiveDateTime.t()
  }
  defstruct [:id, :total, :notes, :created_at]
end

defmodule GetOrderTotalRow do
  @moduledoc "Row type for GetOrderTotal queries."

  @type t :: %__MODULE__{
    total_sum: integer() | nil
  }
  defstruct [:total_sum]
end

defmodule GetUserByIdRow do
  @moduledoc "Row type for GetUserById queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    active: integer(),
    created_at: NaiveDateTime.t()
  }
  defstruct [:id, :name, :email, :active, :created_at]
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

defmodule CreateUserRow do
  @moduledoc "Row type for CreateUser queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    active: integer(),
    created_at: NaiveDateTime.t()
  }
  defstruct [:id, :name, :email, :active, :created_at]
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

defmodule Scythe.Queries do

@spec create_order(DBConnection.conn(), integer(), integer(), String.t() | nil) :: {:ok, %CreateOrderRow{}} | {:error, :not_found} | {:error, term()}
def create_order(conn, user_id, total, notes) do
  case Jamdb.Oracle.query(conn, "INSERT INTO orders (user_id, total, notes) VALUES (:1, :2, :3) RETURNING id, user_id, total, notes, created_at INTO :4, :5, :6, :7, :8", [user_id, total, notes, {:out, :integer}, {:out, :integer}, {:out, :integer}, {:out, :varchar}, {:out, :date}]) do
    {:ok, %{rows: [row | _]}} ->
      [id, user_id, total, notes, created_at] = row
      {:ok, %CreateOrderRow{id: id, user_id: user_id, total: total, notes: notes, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec get_orders_by_user(DBConnection.conn(), integer()) :: {:ok, [%GetOrdersByUserRow{}]} | {:error, term()}
def get_orders_by_user(conn, user_id) do
  case Jamdb.Oracle.query(conn, "SELECT id, total, notes, created_at FROM orders WHERE user_id = :1 ORDER BY created_at DESC", [user_id]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, total, notes, created_at] = row
        %GetOrdersByUserRow{id: id, total: total, notes: notes, created_at: created_at}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec get_order_total(DBConnection.conn(), integer()) :: {:ok, %GetOrderTotalRow{}} | {:error, :not_found} | {:error, term()}
def get_order_total(conn, user_id) do
  case Jamdb.Oracle.query(conn, "SELECT SUM(total) AS total_sum FROM orders WHERE user_id = :1", [user_id]) do
    {:ok, %{rows: [row | _]}} ->
      [total_sum] = row
      {:ok, %GetOrderTotalRow{total_sum: total_sum}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec delete_orders_by_user(DBConnection.conn(), integer()) :: {:ok, non_neg_integer()} | {:error, term()}
def delete_orders_by_user(conn, user_id) do
  case Jamdb.Oracle.query(conn, "DELETE FROM orders WHERE user_id = :1", [user_id]) do
    {:ok, %{num_rows: n}} -> {:ok, n}
    {:error, err} -> {:error, err}
  end
end

@spec get_user_by_id(DBConnection.conn(), integer()) :: {:ok, %GetUserByIdRow{}} | {:error, :not_found} | {:error, term()}
def get_user_by_id(conn, id) do
  case Jamdb.Oracle.query(conn, "SELECT id, name, email, active, created_at FROM users WHERE id = :1", [id]) do
    {:ok, %{rows: [row | _]}} ->
      [id, name, email, active, created_at] = row
      {:ok, %GetUserByIdRow{id: id, name: name, email: email, active: active, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec list_active_users(DBConnection.conn()) :: {:ok, [%ListActiveUsersRow{}]} | {:error, term()}
def list_active_users(conn) do
  case Jamdb.Oracle.query(conn, "SELECT id, name, email FROM users WHERE active = 1", []) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %ListActiveUsersRow{id: id, name: name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec create_user(DBConnection.conn(), String.t(), String.t() | nil, integer()) :: {:ok, %CreateUserRow{}} | {:error, :not_found} | {:error, term()}
def create_user(conn, name, email, active) do
  case Jamdb.Oracle.query(conn, "INSERT INTO users (name, email, active) VALUES (:1, :2, :3) RETURNING id, name, email, active, created_at INTO :4, :5, :6, :7, :8", [name, email, active, {:out, :integer}, {:out, :varchar}, {:out, :varchar}, {:out, :integer}, {:out, :date}]) do
    {:ok, %{rows: [row | _]}} ->
      [id, name, email, active, created_at] = row
      {:ok, %CreateUserRow{id: id, name: name, email: email, active: active, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec update_user_email(DBConnection.conn(), String.t(), integer()) :: :ok | {:error, term()}
def update_user_email(conn, email, id) do
  case Jamdb.Oracle.query(conn, "UPDATE users SET email = :1 WHERE id = :2", [email, id]) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

@spec delete_user(DBConnection.conn(), integer()) :: :ok | {:error, term()}
def delete_user(conn, id) do
  case Jamdb.Oracle.query(conn, "DELETE FROM users WHERE id = :1", [id]) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

@spec search_users(DBConnection.conn(), String.t()) :: {:ok, [%SearchUsersRow{}]} | {:error, term()}
def search_users(conn, name) do
  case Jamdb.Oracle.query(conn, "SELECT id, name, email FROM users WHERE name LIKE :1", [name]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %SearchUsersRow{id: id, name: name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

end
