# nitrocop-config: EnforcedStyle: aggressive

Poll::Shift.where(booth_id: booth_id,
                  officer_id: officer_assignments.pluck(:officer_id),
                                                  ^^^^^ Rails/PluckInWhere: Use `select` instead of `pluck` within `where` query method.
                  date: officer_assignments.pluck(:date))
                                            ^^^^^ Rails/PluckInWhere: Use `select` instead of `pluck` within `where` query method.

where(["(subscribable_id = ? AND subscribable_type = 'Work')
        OR (subscribable_id IN (?) AND subscribable_type = 'User')
       OR (subscribable_id IN (?) AND subscribable_type = 'Series')",
       work.id,
       work.pseuds.pluck(:user_id),
                   ^^^^^ Rails/PluckInWhere: Use `select` instead of `pluck` within `where` query method.
       work.serial_works.pluck(:series_id)])
                         ^^^^^ Rails/PluckInWhere: Use `select` instead of `pluck` within `where` query method.

ActiveStorage::Attachment.where(record_id: templates.map(&:id),
                                record_type: "Template",
                                name: :documents,
                                uuid: templates.flat_map { |t| t.schema.pluck("attachment_uuid") })
                                                                        ^^^^^ Rails/PluckInWhere: Use `select` instead of `pluck` within `where` query method.

joins(:common_taggings).where("filterable_id in (?)", parents.first.is_a?(Integer) ? parents : (parents.respond_to?(:pluck) ? parents.pluck(:id) : parents.collect(&:id)))
                                                                                                                                      ^^^^^ Rails/PluckInWhere: Use `select` instead of `pluck` within `where` query method.
