# frozen_string_literal: true
# scythe:provenance v=0.16.0 backend=ruby-oci8 engine=oracle schema=sch1:51c12e41405f20c2 queries=q1:9b9c257a90458ab4 options=opt1:cbf29ce484222325

require 'oci8'

module Queries
  class RecordNotFound < StandardError; end
  def self.read_lob(value)
    value.nil? ? nil : value.read
  end

  CreateAttachmentRow = Data.define(:id, :order_id, :filename)


  def self.create_attachment(conn, order_id, filename, payload, description)
    cursor = conn.parse("INSERT INTO attachments (order_id, filename, payload, description) VALUES (:1, :2, :3, :4) RETURNING id, order_id, filename INTO :5, :6, :7")
    cursor.bind_param(1, order_id)
    cursor.bind_param(2, filename)
    cursor.bind_param(3, payload)
    cursor.bind_param(4, description)
    cursor.bind_param(5, nil, Integer)
    cursor.bind_param(6, nil, Integer)
    cursor.bind_param(7, nil, String)
    rows_affected = cursor.exec
    raise RecordNotFound, "create_attachment: no row found" if rows_affected.zero?
    CreateAttachmentRow.new(id: cursor[5], order_id: cursor[6], filename: cursor[7])
  end

  GetAttachmentsByOrderRow = Data.define(:id, :order_id, :filename, :payload, :description)


  def self.get_attachments_by_order(conn, order_id)
    cursor = conn.exec("SELECT id, order_id, filename, payload, description FROM attachments WHERE order_id = :1 ORDER BY id", order_id)
    results = []
    while (row = cursor.fetch)
      results << GetAttachmentsByOrderRow.new(id: row[0], order_id: row[1], filename: row[2], payload: read_lob(row[3]), description: read_lob(row[4]))
    end
    results
  end

  GetAttachmentByIdRow = Data.define(:id, :order_id, :filename, :payload, :description)


  def self.get_attachment_by_id(conn, id)
    cursor = conn.exec("SELECT id, order_id, filename, payload, description FROM attachments WHERE id = :1", id)
    row = cursor.fetch
    return nil if row.nil?
    GetAttachmentByIdRow.new(id: row[0], order_id: row[1], filename: row[2], payload: read_lob(row[3]), description: read_lob(row[4]))
  end

  def self.delete_attachments_by_order(conn, order_id)
    conn.exec("DELETE FROM attachments WHERE order_id = :1", order_id)
  end

  CreateOrderRow = Data.define(:id, :user_id, :total, :notes, :created_at)


  def self.create_order(conn, user_id, total, notes)
    cursor = conn.parse("INSERT INTO orders (user_id, total, notes) VALUES (:1, :2, :3) RETURNING id, user_id, total, notes, created_at INTO :4, :5, :6, :7, :8")
    cursor.bind_param(1, user_id)
    cursor.bind_param(2, total)
    cursor.bind_param(3, notes)
    cursor.bind_param(4, nil, Integer)
    cursor.bind_param(5, nil, Integer)
    cursor.bind_param(6, nil, Float)
    cursor.bind_param(7, nil, String)
    cursor.bind_param(8, nil, Time)
    rows_affected = cursor.exec
    raise RecordNotFound, "create_order: no row found" if rows_affected.zero?
    CreateOrderRow.new(id: cursor[4], user_id: cursor[5], total: cursor[6], notes: cursor[7], created_at: cursor[8])
  end

  GetOrdersByUserRow = Data.define(:id, :total, :notes, :created_at)


  def self.get_orders_by_user(conn, user_id)
    cursor = conn.exec("SELECT id, total, notes, created_at FROM orders WHERE user_id = :1 ORDER BY created_at DESC", user_id)
    results = []
    while (row = cursor.fetch)
      results << GetOrdersByUserRow.new(id: row[0], total: row[1], notes: read_lob(row[2]), created_at: row[3])
    end
    results
  end

  GetOrderTotalRow = Data.define(:total_sum)


  def self.get_order_total(conn, user_id)
    cursor = conn.exec("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = :1", user_id)
    row = cursor.fetch
    raise RecordNotFound, "get_order_total: no row found" if row.nil?
    GetOrderTotalRow.new(total_sum: row[0])
  end

  def self.delete_orders_by_user(conn, user_id)
    conn.exec("DELETE FROM orders WHERE user_id = :1", user_id)
  end

  GetUserByIdRow = Data.define(:id, :name, :email, :active, :created_at)


  def self.get_user_by_id(conn, id)
    cursor = conn.exec("SELECT id, name, email, active, created_at FROM users WHERE id = :1", id)
    row = cursor.fetch
    raise RecordNotFound, "get_user_by_id: no row found" if row.nil?
    GetUserByIdRow.new(id: row[0], name: row[1], email: row[2], active: row[3], created_at: row[4])
  end

  ListActiveUsersRow = Data.define(:id, :name, :email)


  def self.list_active_users(conn)
    cursor = conn.exec("SELECT id, name, email FROM users WHERE active = 1")
    results = []
    while (row = cursor.fetch)
      results << ListActiveUsersRow.new(id: row[0], name: row[1], email: row[2])
    end
    results
  end

  CreateUserRow = Data.define(:id, :name, :email, :active, :created_at)


  def self.create_user(conn, name, email, active)
    cursor = conn.parse("INSERT INTO users (name, email, active) VALUES (:1, :2, :3) RETURNING id, name, email, active, created_at INTO :4, :5, :6, :7, :8")
    cursor.bind_param(1, name)
    cursor.bind_param(2, email)
    cursor.bind_param(3, active)
    cursor.bind_param(4, nil, Integer)
    cursor.bind_param(5, nil, String)
    cursor.bind_param(6, nil, String)
    cursor.bind_param(7, nil, Integer)
    cursor.bind_param(8, nil, Time)
    rows_affected = cursor.exec
    raise RecordNotFound, "create_user: no row found" if rows_affected.zero?
    CreateUserRow.new(id: cursor[4], name: cursor[5], email: cursor[6], active: cursor[7], created_at: cursor[8])
  end

  def self.update_user_email(conn, email, id)
    conn.exec("UPDATE users SET email = :1 WHERE id = :2", email, id)
    nil
  end

  def self.delete_user(conn, id)
    conn.exec("DELETE FROM users WHERE id = :1", id)
    nil
  end

  SearchUsersRow = Data.define(:id, :name, :email)


  def self.search_users(conn, name)
    cursor = conn.exec("SELECT id, name, email FROM users WHERE name LIKE :1", name)
    results = []
    while (row = cursor.fetch)
      results << SearchUsersRow.new(id: row[0], name: row[1], email: row[2])
    end
    results
  end

end
