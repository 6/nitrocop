create_table :users do |t|
^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/CreateTableWithTimestamps: Add `t.timestamps` to `create_table` block.
  t.string :name
  t.string :email
end

create_table :posts do |t|
^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/CreateTableWithTimestamps: Add `t.timestamps` to `create_table` block.
  t.string :title
  t.text :body
  t.references :user
end

create_table :comments do |t|
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/CreateTableWithTimestamps: Add `t.timestamps` to `create_table` block.
  t.text :content
end

create_table :audit_logs
^^^^^^^^^^^^^^^^^^^^^^^^ Rails/CreateTableWithTimestamps: Add `t.timestamps` to `create_table` block.

create_table :events, &:extension_columns
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/CreateTableWithTimestamps: Add `t.timestamps` to `create_table` block.

class ActsAsTaggableOnMigration < ActiveRecord::Migration[4.2]
  def self.up
    create_table :tags do |t|
    ^ Rails/CreateTableWithTimestamps: Add `t.timestamps` to `create_table` block.
      t.string :name
    end
  end
end
