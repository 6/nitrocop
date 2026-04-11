# nitrocop-filename: db/migrate/001_create_table_with_timestamps.rb

create_table :users do |t|
  t.string :name
  t.string :email
  t.timestamps
end

create_table :events do |t|
  t.string :name
  t.datetime :created_at, null: false
end

create_table :join_table, id: false do |t|
  t.integer :user_id
  t.integer :article_id
end
