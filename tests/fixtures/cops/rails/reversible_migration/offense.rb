class ExampleMigration < ActiveRecord::Migration[7.0]
  def change
    execute "ALTER TABLE pages ADD UNIQUE idx (page_id)"
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/ReversibleMigration: execute is not reversible.
  end
end

class DropMigration < ActiveRecord::Migration[7.0]
  def change
    drop_table :users
    ^^^^^^^^^^^^^^^^^ Rails/ReversibleMigration: drop_table(without block) is not reversible.
  end
end

class RemoveMigration < ActiveRecord::Migration[7.0]
  def change
    remove_column(:suppliers, :qualification)
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/ReversibleMigration: remove_column(without type) is not reversible.
  end
end

class ChangeColumnMigration < ActiveRecord::Migration[7.0]
  def change
    change_column(:posts, :state, :string)
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/ReversibleMigration: change_column is not reversible.
  end
end

class AddTokensToCourseLessonPlanItems < ActiveRecord::Migration[4.2]
  def change
    assessment.lesson_plan_item.update_column(:start_at, assessment.start_at.change(usec: 0))
                                                         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/ReversibleMigration: change(without :from and :to) is not reversible.

    assessment.lesson_plan_item.update_column(:end_at, assessment.end_at.change(usec: 0))
                                                       ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/ReversibleMigration: change(without :from and :to) is not reversible.
  end
end

class Fix21 < ActiveRecord::Migration[6.0]
  def change
    change_table :swars_memberships do |t|
      if t.column_exists?(:obt_think_avg)
        t.remove :obt_think_avg
        ^^^^^^^^^^^^^^^^^^^^^^^ Rails/ReversibleMigration: t.remove (without type) is not reversible.
      end
    end
  end
end

class FixAuthInfos2 < ActiveRecord::Migration[6.0]
  def change
    change_table :auth_infos do |t|
      t.remove :meta_info rescue nil
      ^^^^^^^^^^^^^^^^^^^ Rails/ReversibleMigration: t.remove (without type) is not reversible.
    end
  end
end

class RemoveActiveFromProfileField < ActiveRecord::Migration[6.0]
  def change
    def change
      safety_assured { remove_column :profile_fields, :active }
                       ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/ReversibleMigration: remove_column(without type) is not reversible.
    end
  end
end
