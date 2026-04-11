# nitrocop-filename: db/migrate/001_create_table_with_timestamps.rb

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

class CreateOrganisations < ActiveRecord::Migration[4.2]
  def change
    create_table :organisations do |t|
      t.string :login
      t.string :avatar_url
      t.integer :github_id

      t.timestamps null: false
    end

    create_table :organisations_users do |t|
    ^ Rails/CreateTableWithTimestamps: Add `t.timestamps` to `create_table` block.
      t.integer :user_id
      t.integer :organisation_id
    end
  end
end

class ActsAsTaggableOnMigration < ActiveRecord::Migration[4.2]
  def self.up
    create_table :tags do |t|
    ^ Rails/CreateTableWithTimestamps: Add `t.timestamps` to `create_table` block.
      t.string :name
    end

    create_table :taggings do |t|
    ^ Rails/CreateTableWithTimestamps: Add `t.timestamps` to `create_table` block.
      t.references :tag
    end
  end
end

ActiveRecord::Schema[7.1].define(version: 2021_12_12_143544) do
  create_table "organisations_users", id: :serial, force: :cascade do |t|
  ^ Rails/CreateTableWithTimestamps: Add `t.timestamps` to `create_table` block.
    t.integer "user_id"
    t.integer "organisation_id"
  end
end
