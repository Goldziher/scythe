# scythe:provenance v=0.17.0 backend=elixir-jamdb engine=oracle schema=sch2:51c12e41405f20c2 queries=q1:9b9c257a90458ab4 options=opt1:cbf29ce484222325
defmodule CreateAttachmentRow do
  @moduledoc "Row type for CreateAttachment queries."

  @type t :: %__MODULE__{
    id: integer(),
    order_id: integer(),
    filename: String.t()
  }
  defstruct [:id, :order_id, :filename]
end

defmodule GetAttachmentsByOrderRow do
  @moduledoc "Row type for GetAttachmentsByOrder queries."

  @type t :: %__MODULE__{
    id: integer(),
    order_id: integer(),
    filename: String.t(),
    payload: binary(),
    description: String.t() | nil
  }
  defstruct [:id, :order_id, :filename, :payload, :description]
end

defmodule GetAttachmentByIdRow do
  @moduledoc "Row type for GetAttachmentById queries."

  @type t :: %__MODULE__{
    id: integer(),
    order_id: integer(),
    filename: String.t(),
    payload: binary(),
    description: String.t() | nil
  }
  defstruct [:id, :order_id, :filename, :payload, :description]
end

defmodule CreateOrderRow do
  @moduledoc "Row type for CreateOrder queries."

  @type t :: %__MODULE__{
    id: integer(),
    user_id: integer(),
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

@spec create_attachment(DBConnection.conn(), integer(), String.t(), binary(), String.t() | nil) :: {:ok, %CreateAttachmentRow{}} | {:error, :not_found} | {:error, term()}
def create_attachment(conn, order_id, filename, payload, description) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "INSERT INTO attachments (order_id, filename, payload, description) VALUES (:1, :2, :3, :4) RETURNING id, order_id, filename INTO :5, :6, :7"}, [order_id, filename, payload, description, {:out, :integer}, {:out, :integer}, {:out, :varchar}]) do
    {:ok, _query, %{rows: rows}} when rows != [] ->
      [id, order_id, filename] = Enum.map(rows, &hd/1)
      {:ok, %CreateAttachmentRow{id: (if is_float(id), do: trunc(id), else: id), order_id: (if is_float(order_id), do: trunc(order_id), else: order_id), filename: filename}}
    {:ok, _query, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec get_attachments_by_order(DBConnection.conn(), integer()) :: {:ok, [%GetAttachmentsByOrderRow{}]} | {:error, term()}
def get_attachments_by_order(conn, order_id) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "SELECT id, order_id, filename, payload, description FROM attachments WHERE order_id = :1 ORDER BY id"}, [order_id]) do
    {:ok, _query, %{rows: rows}} when is_list(rows) ->
      results = Enum.map(rows, fn row ->
        [id, order_id, filename, payload, description] = row
        %GetAttachmentsByOrderRow{id: (if is_float(id), do: trunc(id), else: id), order_id: (if is_float(order_id), do: trunc(order_id), else: order_id), filename: filename, payload: payload, description: description}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec get_attachment_by_id(DBConnection.conn(), integer()) :: {:ok, %GetAttachmentByIdRow{} | nil} | {:error, term()}
def get_attachment_by_id(conn, id) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "SELECT id, order_id, filename, payload, description FROM attachments WHERE id = :1"}, [id]) do
    {:ok, _query, %{rows: [row | _]}} ->
      [id, order_id, filename, payload, description] = row
      {:ok, %GetAttachmentByIdRow{id: (if is_float(id), do: trunc(id), else: id), order_id: (if is_float(order_id), do: trunc(order_id), else: order_id), filename: filename, payload: payload, description: description}}
    {:ok, _query, %{rows: []}} -> {:ok, nil}
    {:error, err} -> {:error, err}
  end
end

@spec delete_attachments_by_order(DBConnection.conn(), integer()) :: {:ok, non_neg_integer()} | {:error, term()}
def delete_attachments_by_order(conn, order_id) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "DELETE FROM attachments WHERE order_id = :1"}, [order_id]) do
    {:ok, _query, %{num_rows: n}} -> {:ok, n}
    {:error, err} -> {:error, err}
  end
end

@spec create_order(DBConnection.conn(), integer(), Decimal.t(), String.t() | nil) :: {:ok, %CreateOrderRow{}} | {:error, :not_found} | {:error, term()}
def create_order(conn, user_id, total, notes) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "INSERT INTO orders (user_id, total, notes) VALUES (:1, :2, :3) RETURNING id, user_id, total, notes, created_at INTO :4, :5, :6, :7, :8"}, [user_id, total, notes, {:out, :integer}, {:out, :integer}, {:out, :number}, {:out, :varchar}, {:out, :date}]) do
    {:ok, _query, %{rows: rows}} when rows != [] ->
      [id, user_id, total, notes, created_at] = Enum.map(rows, &hd/1)
      {:ok, %CreateOrderRow{id: (if is_float(id), do: trunc(id), else: id), user_id: (if is_float(user_id), do: trunc(user_id), else: user_id), total: (if is_float(total), do: Decimal.from_float(total), else: total), notes: notes, created_at: created_at}}
    {:ok, _query, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec get_orders_by_user(DBConnection.conn(), integer()) :: {:ok, [%GetOrdersByUserRow{}]} | {:error, term()}
def get_orders_by_user(conn, user_id) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "SELECT id, total, notes, created_at FROM orders WHERE user_id = :1 ORDER BY created_at DESC"}, [user_id]) do
    {:ok, _query, %{rows: rows}} when is_list(rows) ->
      results = Enum.map(rows, fn row ->
        [id, total, notes, created_at] = row
        %GetOrdersByUserRow{id: (if is_float(id), do: trunc(id), else: id), total: (if is_float(total), do: Decimal.from_float(total), else: total), notes: notes, created_at: created_at}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec get_order_total(DBConnection.conn(), integer()) :: {:ok, %GetOrderTotalRow{}} | {:error, :not_found} | {:error, term()}
def get_order_total(conn, user_id) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "SELECT SUM(total) AS total_sum FROM orders WHERE user_id = :1"}, [user_id]) do
    {:ok, _query, %{rows: [row | _]}} ->
      [total_sum] = row
      {:ok, %GetOrderTotalRow{total_sum: (if is_float(total_sum), do: Decimal.from_float(total_sum), else: total_sum)}}
    {:ok, _query, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec delete_orders_by_user(DBConnection.conn(), integer()) :: {:ok, non_neg_integer()} | {:error, term()}
def delete_orders_by_user(conn, user_id) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "DELETE FROM orders WHERE user_id = :1"}, [user_id]) do
    {:ok, _query, %{num_rows: n}} -> {:ok, n}
    {:error, err} -> {:error, err}
  end
end

@spec get_user_by_id(DBConnection.conn(), integer()) :: {:ok, %GetUserByIdRow{}} | {:error, :not_found} | {:error, term()}
def get_user_by_id(conn, id) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "SELECT id, name, email, active, created_at FROM users WHERE id = :1"}, [id]) do
    {:ok, _query, %{rows: [row | _]}} ->
      [id, name, email, active, created_at] = row
      {:ok, %GetUserByIdRow{id: (if is_float(id), do: trunc(id), else: id), name: name, email: email, active: (if is_float(active), do: trunc(active), else: active), created_at: created_at}}
    {:ok, _query, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec list_active_users(DBConnection.conn()) :: {:ok, [%ListActiveUsersRow{}]} | {:error, term()}
def list_active_users(conn) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "SELECT id, name, email FROM users WHERE active = 1"}, []) do
    {:ok, _query, %{rows: rows}} when is_list(rows) ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %ListActiveUsersRow{id: (if is_float(id), do: trunc(id), else: id), name: name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec create_user(DBConnection.conn(), String.t(), String.t() | nil, integer()) :: {:ok, %CreateUserRow{}} | {:error, :not_found} | {:error, term()}
def create_user(conn, name, email, active) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "INSERT INTO users (name, email, active) VALUES (:1, :2, :3) RETURNING id, name, email, active, created_at INTO :4, :5, :6, :7, :8"}, [name, email, active, {:out, :integer}, {:out, :varchar}, {:out, :varchar}, {:out, :integer}, {:out, :date}]) do
    {:ok, _query, %{rows: rows}} when rows != [] ->
      [id, name, email, active, created_at] = Enum.map(rows, &hd/1)
      {:ok, %CreateUserRow{id: (if is_float(id), do: trunc(id), else: id), name: name, email: email, active: (if is_float(active), do: trunc(active), else: active), created_at: created_at}}
    {:ok, _query, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec update_user_email(DBConnection.conn(), String.t(), integer()) :: :ok | {:error, term()}
def update_user_email(conn, email, id) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "UPDATE users SET email = :1 WHERE id = :2"}, [email, id]) do
    {:ok, _query, _result} -> :ok
    {:error, err} -> {:error, err}
  end
end

@spec delete_user(DBConnection.conn(), integer()) :: :ok | {:error, term()}
def delete_user(conn, id) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "DELETE FROM users WHERE id = :1"}, [id]) do
    {:ok, _query, _result} -> :ok
    {:error, err} -> {:error, err}
  end
end

@spec search_users(DBConnection.conn(), String.t()) :: {:ok, [%SearchUsersRow{}]} | {:error, term()}
def search_users(conn, name) do
  case DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: "SELECT id, name, email FROM users WHERE name LIKE :1"}, [name]) do
    {:ok, _query, %{rows: rows}} when is_list(rows) ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %SearchUsersRow{id: (if is_float(id), do: trunc(id), else: id), name: name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

end
