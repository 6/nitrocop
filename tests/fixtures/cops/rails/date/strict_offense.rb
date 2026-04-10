# nitrocop-config: EnforcedStyle: strict

Date.today
     ^^^^^ Rails/Date: Do not use `Date.today` without zone. Use `Time.zone.today` instead.

Date.yesterday
     ^^^^^^^^^ Rails/Date: Do not use `Date.yesterday` without zone. Use `Time.zone.yesterday` instead.

Date.tomorrow
     ^^^^^^^^ Rails/Date: Do not use `Date.tomorrow` without zone. Use `Time.zone.tomorrow` instead.

Date.current
     ^^^^^^^ Rails/Date: Do not use `Date.current` without zone. Use `Time.zone.today` instead.

::Date.today
       ^^^^^ Rails/Date: Do not use `Date.today` without zone. Use `Time.zone.today` instead.

value.to_time_in_current_zone
      ^^^^^^^^^^^^^^^^^^^^^^^^ Rails/Date: `to_time_in_current_zone` is deprecated. Use `in_time_zone` instead.
