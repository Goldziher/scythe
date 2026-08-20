# frozen_string_literal: true
# scythe:provenance v=0.16.1 backend=ruby-trilogy engine=mysql schema=sch1:4332a9c33cb39297 queries=q1:f928696deb211f90 options=opt1:cbf29ce484222325

require "json"
require "bigdecimal/util"

module Queries
  class RecordNotFound < StandardError; end

  module UsersStatus
    ACTIVE = "active"
    INACTIVE = "inactive"
    BANNED = "banned"
    ALL = [ACTIVE, INACTIVE, BANNED].freeze
  end

  def self.create_order(client, user_id, total, notes)
    client.query("INSERT INTO orders (user_id, total, notes) VALUES (#{user_id}, #{total}, '#{client.escape(notes.to_s)}')")
    nil
  end

  GetLastInsertOrderRow = Data.define(:id, :user_id, :total, :notes, :created_at)


  def self.get_last_insert_order(client)
    results = client.query("SELECT id, user_id, total, notes, created_at FROM orders WHERE id = LAST_INSERT_ID()")
    row = results.first
    raise RecordNotFound, "get_last_insert_order: no row found" if row.nil?
    GetLastInsertOrderRow.new(id: row[0].to_i, user_id: row[1].to_i, total: row[2].to_d, notes: row[3]&.then { |v| v }, created_at: row[4])
  end

  GetOrdersByUserRow = Data.define(:id, :total, :notes, :created_at)


  def self.get_orders_by_user(client, user_id)
    results = client.query("SELECT id, total, notes, created_at FROM orders WHERE user_id = #{user_id} ORDER BY created_at DESC")
    results.map do |row|
      GetOrdersByUserRow.new(id: row[0].to_i, total: row[1].to_d, notes: row[2]&.then { |v| v }, created_at: row[3])
    end
  end

  GetOrderTotalRow = Data.define(:total_sum)


  def self.get_order_total(client, user_id)
    results = client.query("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = #{user_id}")
    row = results.first
    raise RecordNotFound, "get_order_total: no row found" if row.nil?
    GetOrderTotalRow.new(total_sum: row[0]&.then { |v| v.to_d })
  end

  def self.delete_orders_by_user(client, user_id)
    client.query("DELETE FROM orders WHERE user_id = #{user_id}")
    client.affected_rows
  end

  GetUserByIdRow = Data.define(:id, :name, :email, :status, :created_at)


  def self.get_user_by_id(client, id)
    results = client.query("SELECT id, name, email, status, created_at FROM users WHERE id = #{id}")
    row = results.first
    raise RecordNotFound, "get_user_by_id: no row found" if row.nil?
    GetUserByIdRow.new(id: row[0].to_i, name: row[1], email: row[2]&.then { |v| v }, status: row[3], created_at: row[4])
  end

  ListActiveUsersRow = Data.define(:id, :name, :email)


  def self.list_active_users(client, status)
    results = client.query("SELECT id, name, email FROM users WHERE status = '#{client.escape(status.to_s)}'")
    results.map do |row|
      ListActiveUsersRow.new(id: row[0].to_i, name: row[1], email: row[2]&.then { |v| v })
    end
  end

  def self.create_user(client, name, email, status)
    client.query("INSERT INTO users (name, email, status) VALUES ('#{client.escape(name.to_s)}', '#{client.escape(email.to_s)}', '#{client.escape(status.to_s)}')")
    nil
  end

  GetLastInsertUserRow = Data.define(:id, :name, :email, :status, :created_at)


  def self.get_last_insert_user(client)
    results = client.query("SELECT id, name, email, status, created_at FROM users WHERE id = LAST_INSERT_ID()")
    row = results.first
    raise RecordNotFound, "get_last_insert_user: no row found" if row.nil?
    GetLastInsertUserRow.new(id: row[0].to_i, name: row[1], email: row[2]&.then { |v| v }, status: row[3], created_at: row[4])
  end

  def self.update_user_email(client, email, id)
    client.query("UPDATE users SET email = '#{client.escape(email.to_s)}' WHERE id = #{id}")
    nil
  end

  def self.delete_user(client, id)
    client.query("DELETE FROM users WHERE id = #{id}")
    nil
  end

  SearchUsersRow = Data.define(:id, :name, :email)


  def self.search_users(client, name)
    results = client.query("SELECT id, name, email FROM users WHERE name LIKE '#{client.escape(name.to_s)}'")
    results.map do |row|
      SearchUsersRow.new(id: row[0].to_i, name: row[1], email: row[2]&.then { |v| v })
    end
  end

end
