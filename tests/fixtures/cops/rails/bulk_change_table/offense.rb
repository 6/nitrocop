# nitrocop-filename: db/migrate/001_example.rb
def change
  change_table :users do |t|
  ^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can combine alter queries using `bulk: true` options.
    t.string :name, null: false
    t.string :address, null: true
  end
end

def change
  change_table :orders do |t|
  ^^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can combine alter queries using `bulk: true` options.
    t.index :name
    t.index :address
  end
end

def change
  add_column :users, :name, :string, null: false
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can use `change_table :users, bulk: true` to combine alter queries.
  remove_column :users, :nickname
end

def change
  add_column :users, :twitter_token, :string
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can use `change_table :users, bulk: true` to combine alter queries.
  add_column :users, :twitter_secret, :string
end

def change
  add_column :users, :confirmation_token, :string
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can use `change_table :users, bulk: true` to combine alter queries.
  add_column :users, :confirmed_at, :datetime
end

def change
  add_column :users, :name, :string
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can use `change_table :users, bulk: true` to combine alter queries.
  add_column :users, :blog, :string
  add_column :users, :location, :string
end

def change
  add_column :users, :lat, :decimal, precision: 8, scale: 6
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can use `change_table :users, bulk: true` to combine alter queries.
  add_column :users, :lng, :decimal, precision: 9, scale: 6
end

def change
  add_column :projects, :featured, :boolean, :default => false
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can use `change_table :projects, bulk: true` to combine alter queries.
  add_column :projects, :avatar_url, :string
end

def change
  add_column :projects, :last_scored, :datetime
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can use `change_table :projects, bulk: true` to combine alter queries.
  add_column :projects, :fork, :boolean
  add_column :projects, :github_id, :bigint
end

class AddTypeAndUserIdToLogEntry < ActiveRecord::Migration[5.2]
  def change
    add_column :log_entries, :type, :string
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can use `change_table :log_entries, bulk: true` to combine alter queries.
    add_column :log_entries, :user_id, :integer

    LogEntry.find_each do |log_entry|
      log_entry.type = "CharacterLogEntry"
      log_entry.save!
    end
  end
end

change_table :users do |t|
^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can combine alter queries using `bulk: true` options.
  t.string :email, null: false
  t.string :encrypted_password, null: false
end

class AddPublisherToSubmissions < ActiveRecord::Migration[4.2]
  change_table :course_assessment_submissions do |t|
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can combine alter queries using `bulk: true` options.
    t.integer :publisher_id, foreign_key: { references: :users }
    t.datetime :published_at
  end
end

class RenameSocialMediaColumnsOnAffiliates < ActiveRecord::Migration
  def self.up
    change_table :affiliates do |t|
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can combine alter queries using `bulk: true` options.
      t.rename :facebook_username, :facebook_handle
      t.rename :twitter_username, :twitter_handle
    end
  end

  def self.down
    change_table :affiliates do |t|
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/BulkChangeTable: You can combine alter queries using `bulk: true` options.
      t.rename :twitter_handle, :twitter_username
      t.rename :facebook_handle, :facebook_username
    end
  end
end
