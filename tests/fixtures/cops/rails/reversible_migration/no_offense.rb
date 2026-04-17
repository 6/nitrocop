class CreateUsers < ActiveRecord::Migration[7.0]
  def change
    create_table :users do |t|
      t.string :name
    end
  end
end

class ReversibleExample < ActiveRecord::Migration[7.0]
  def change
    reversible do |dir|
      dir.up do
        execute "ALTER TABLE pages ADD UNIQUE idx (page_id)"
      end
      dir.down do
        execute "ALTER TABLE pages DROP INDEX idx"
      end
    end
  end
end

class UpOnlyExample < ActiveRecord::Migration[7.0]
  def change
    up_only { execute "UPDATE posts SET published = 'true'" }
  end
end

class RemoveWithType < ActiveRecord::Migration[7.0]
  def change
    remove_column(:suppliers, :qualification, :string)
  end
end

class DropWithBlock < ActiveRecord::Migration[7.0]
  def change
    drop_table :users do |t|
      t.string :name
    end
  end
end

class DefaultWithFromTo < ActiveRecord::Migration[7.0]
  def change
    change_column_default(:posts, :state, from: nil, to: "draft")
  end
end

class RemoveAffiliateFromDailyLeftNavStats < ActiveRecord::Migration
  def change
    remove_column :daily_left_nav_stats, :affiliate_id
  end
end

class RemoveNullConstraintOnTagging < ActiveRecord::Migration
  def change
    change_column :taggings, :entry_id, :integer, null: true
  end
end

class RemoveTripPositionsIntermediate < ActiveRecord::Migration[7.1]
  def change
    safety_assured do
      drop_table :trip_positions_intermediate
    end
  end
end

class TitleToName < ActiveRecord::Migration[6.0]
  def change
    %i[descriptions images statuses taggings tags].each do |table|
      drop_table table
    end
  end
end

class RemoveScavengerHuntTables < ActiveRecord::Migration[6.0]
  def change
    %i[answers clues games hints locations players survey_answers survey_questions].each do |table|
      drop_table "scavenger_hunt_#{table}"
    end
  end
end

class Fix61 < ActiveRecord::Migration[6.0]
  def change
    [
      :emox_emotion_folders,
      :emox_finals,
      :emox_judges,
      :emox_lineages,
      :emox_ox_marks,
      :emox_rules,
      :emox_seasons,
      :emox_skills,
      :emox_source_abouts,
      :transient_aggregates,
    ].each do |e|
      drop_table e, if_exists: true
    end
  end
end
