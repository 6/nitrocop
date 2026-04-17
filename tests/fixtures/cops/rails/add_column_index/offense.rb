add_column :table, :column, :integer, index: true
                                      ^ Rails/AddColumnIndex: `add_column` does not accept an `index` key, use `add_index` instead.
add_column :users, :group_id, :integer, index: true
                                        ^ Rails/AddColumnIndex: `add_column` does not accept an `index` key, use `add_index` instead.
add_column :posts, :category_id, :bigint, null: false, index: { unique: true }
                                                       ^ Rails/AddColumnIndex: `add_column` does not accept an `index` key, use `add_index` instead.

add_column :course_assessment_answer_programming, :codaveri_feedback_job_id, :uuid,
               index: :unique, comment: 'The ID of the codaveri code feedback job'
               ^ Rails/AddColumnIndex: `add_column` does not accept an `index` key, use `add_index` instead.

add_column :finished_at, :timestamptz,
                 index: true, # to quickly find the null
                 ^ Rails/AddColumnIndex: `add_column` does not accept an `index` key, use `add_index` instead.
                 null: true # will be enforced by a trigger

add_column :users,
           :discord_user_id,
           :string,
           null: true,
           default: nil,
           index: true
           ^ Rails/AddColumnIndex: `add_column` does not accept an `index` key, use `add_index` instead.
