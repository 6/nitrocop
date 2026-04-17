add_column :users, :active, :boolean
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/ThreeStateBooleanColumn: Boolean columns should always have a default value and a `NOT NULL` constraint.
t.boolean :active
^^^^^^^^^^^^^^^^^ Rails/ThreeStateBooleanColumn: Boolean columns should always have a default value and a `NOT NULL` constraint.
add_column :posts, :published, :boolean
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/ThreeStateBooleanColumn: Boolean columns should always have a default value and a `NOT NULL` constraint.
add_column :users, :admin, :boolean, default: false
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/ThreeStateBooleanColumn: Boolean columns should always have a default value and a `NOT NULL` constraint.
t.boolean :active, null: false
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/ThreeStateBooleanColumn: Boolean columns should always have a default value and a `NOT NULL` constraint.

class AddFormerUserToUser < ActiveRecord::Migration[6.0]
  def change
    add_column :users, :former_user, :boolean, default: false
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/ThreeStateBooleanColumn: Boolean columns should always have a default value and a `NOT NULL` constraint.
    change_column_null :users, :former_user, false, false
  end
end

class CreateGtfsEngineCalendars < ActiveRecord::Migration[4.2]
  TABLE = :gtfs_engine_calendars

  def change
    create_table TABLE do |t|
      t.boolean :monday,     null: false
      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/ThreeStateBooleanColumn: Boolean columns should always have a default value and a `NOT NULL` constraint.
    end
  end
end
