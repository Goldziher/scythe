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
  query = %Jamdb.Oracle.Query{statement: "INSERT INTO orders (user_id, total, notes) VALUES (:1, :2, :3) RETURNING id, user_id, total, notes, created_at INTO :4, :5, :6, :7, :8"}
  case DBConnection.execute(conn, query, [user_id, total, notes], out: [:integer, :integer, :integer, :varchar, :date]) do
    {:ok, _, %{rows: [[id], [ret_user_id], [ret_total], [ret_notes], [created_at]]}} ->
      {:ok, %CreateOrderRow{id: id, user_id: ret_user_id, total: ret_total, notes: ret_notes, created_at: created_at}}
    {:ok, _, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec get_orders_by_user(DBConnection.conn(), integer()) :: {:ok, [%GetOrdersByUserRow{}]} | {:error, term()}
def get_orders_by_user(conn, user_id) do
  query = %Jamdb.Oracle.Query{statement: "SELECT id, total, notes, created_at FROM orders WHERE user_id = :1 ORDER BY created_at DESC"}
  case DBConnection.execute(conn, query, [user_id], []) do
    {:ok, _, %{rows: rows}} ->
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
  query = %Jamdb.Oracle.Query{statement: "SELECT SUM(total) AS total_sum FROM orders WHERE user_id = :1"}
  case DBConnection.execute(conn, query, [user_id], []) do
    {:ok, _, %{rows: [row | _]}} ->
      [total_sum] = row
      {:ok, %GetOrderTotalRow{total_sum: total_sum}}
    {:ok, _, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec delete_orders_by_user(DBConnection.conn(), integer()) :: {:ok, non_neg_integer()} | {:error, term()}
def delete_orders_by_user(conn, user_id) do
  query = %Jamdb.Oracle.Query{statement: "DELETE FROM orders WHERE user_id = :1"}
  case DBConnection.execute(conn, query, [user_id], []) do
    {:ok, _, %{num_rows: n}} -> {:ok, n}
    {:error, err} -> {:error, err}
  end
end

@spec get_user_by_id(DBConnection.conn(), integer()) :: {:ok, %GetUserByIdRow{}} | {:error, :not_found} | {:error, term()}
def get_user_by_id(conn, id) do
  query = %Jamdb.Oracle.Query{statement: "SELECT id, name, email, active, created_at FROM users WHERE id = :1"}
  case DBConnection.execute(conn, query, [id], []) do
    {:ok, _, %{rows: [row | _]}} ->
      [ret_id, name, email, active, created_at] = row
      {:ok, %GetUserByIdRow{id: ret_id, name: name, email: email, active: active, created_at: created_at}}
    {:ok, _, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec list_active_users(DBConnection.conn()) :: {:ok, [%ListActiveUsersRow{}]} | {:error, term()}
def list_active_users(conn) do
  query = %Jamdb.Oracle.Query{statement: "SELECT id, name, email FROM users WHERE active = 1"}
  case DBConnection.execute(conn, query, [], []) do
    {:ok, _, %{rows: rows}} ->
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
  query = %Jamdb.Oracle.Query{statement: "INSERT INTO users (name, email, active) VALUES (:1, :2, :3) RETURNING id, name, email, active, created_at INTO :4, :5, :6, :7, :8"}
  case DBConnection.execute(conn, query, [name, email, active], out: [:integer, :varchar, :varchar, :integer, :date]) do
    {:ok, _, %{rows: [[id], [ret_name], [ret_email], [ret_active], [created_at]]}} ->
      {:ok, %CreateUserRow{id: id, name: ret_name, email: ret_email, active: ret_active, created_at: created_at}}
    {:ok, _, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec update_user_email(DBConnection.conn(), String.t(), integer()) :: :ok | {:error, term()}
def update_user_email(conn, email, id) do
  query = %Jamdb.Oracle.Query{statement: "UPDATE users SET email = :1 WHERE id = :2"}
  case DBConnection.execute(conn, query, [email, id], []) do
    {:ok, _, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

@spec delete_user(DBConnection.conn(), integer()) :: :ok | {:error, term()}
def delete_user(conn, id) do
  query = %Jamdb.Oracle.Query{statement: "DELETE FROM users WHERE id = :1"}
  case DBConnection.execute(conn, query, [id], []) do
    {:ok, _, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

@spec search_users(DBConnection.conn(), String.t()) :: {:ok, [%SearchUsersRow{}]} | {:error, term()}
def search_users(conn, name) do
  query = %Jamdb.Oracle.Query{statement: "SELECT id, name, email FROM users WHERE name LIKE :1"}
  case DBConnection.execute(conn, query, [name], []) do
    {:ok, _, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, ret_name, email] = row
        %SearchUsersRow{id: id, name: ret_name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

end
