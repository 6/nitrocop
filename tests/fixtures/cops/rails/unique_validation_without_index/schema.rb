ActiveRecord::Schema[7.0].define(version: 2024_01_01) do
  create_table "users", force: :cascade do |t|
    t.string "account"
    t.string "email"
    t.string "username"
    t.string "name"
    t.bigint "organization_id"
    t.index ["email"], unique: true
  end

  create_table "service_statuses", force: :cascade do |t|
    t.string "name"
    t.string "permalink"
  end

  create_table "budget_content_blocks", force: :cascade do |t|
    t.string "locale"
    t.bigint "heading_id"
  end

  create_table "budget_indexed_content_blocks", force: :cascade do |t|
    t.string "locale"
    t.bigint "heading_id"
    t.index ["heading_id", "locale"], unique: true
  end

  create_table "wkbk_books", force: :cascade do |t|
    t.string "title"
    t.bigint "user_id"
  end

  create_table "dns_aliases", force: :cascade do |t|
    t.string "name"
  end
end
