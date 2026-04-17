add_column :users, :active, :boolean, null: false, default: false
t.boolean :active, null: false, default: false
add_column :users, :name, :string
t.string :name
t.boolean :enabled, null: false, default: false
add_column :posts, :visible, :boolean, default: true, null: false

# Migration with change_column_null (add_column form)
class AddFoo < ActiveRecord::Migration[7.0]
  def change
    add_column :users, :foo, :boolean
    change_column_null :users, :foo, false
  end
end

# Migration with change_column_null (create_table form)
class CreatePosts < ActiveRecord::Migration[7.0]
  def change
    create_table :posts do |t|
      t.boolean :active
      t.string :title
    end
    change_column_null :posts, :active, false
  end
end

# Migration with change_column_null (change_table t.column form)
class UpdateUsers < ActiveRecord::Migration[7.0]
  def change
    change_table :users do |t|
      t.column :verified, :boolean
    end
    change_column_null :users, :verified, false
  end
end

class AddFeaturesToIndividualPlans < ActiveRecord::Migration[4.2]
  def change
    with_options default: true, null: false do |table|
      table.add_column :individual_plans, :includes_exercises, :boolean
    end
  end
end

class CreateFeatures < ActiveRecord::Migration
  def self.up
    create_table :features do |t|
      t.boolean :published, :default => false, null: false
      t.boolean :published_at, :datetime, :default => nil
    end
  end
end

# `default: nil` counts as having a default for this cop — matches RuboCop's
# `(pair (sym :default) !nil?)` pattern, where `!nil?` applies to the AST node
# object (never Ruby-nil), not the literal value.
class CreateQueries < ActiveRecord::Migration[5.0]
  def change
    create_table :queries do |t|
      t.boolean :include_subprojects, null: false, default: nil
    end
  end
end
