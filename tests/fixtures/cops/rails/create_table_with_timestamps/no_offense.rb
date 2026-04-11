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

create_table :template_auth_lookup, {:id => false} do |t|
  t.integer :user_id
  t.integer :id, primary_key: true
end
