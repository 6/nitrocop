Time.zone = "Eastern"
^^^^^^^^^^^^^^^^^^^^^ Rails/TimeZoneAssignment: Use `Time.use_zone` with block instead of `Time.zone=`.

Time.zone = "UTC"
^^^^^^^^^^^^^^^^^ Rails/TimeZoneAssignment: Use `Time.use_zone` with block instead of `Time.zone=`.

Time.zone = user.time_zone
^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/TimeZoneAssignment: Use `Time.use_zone` with block instead of `Time.zone=`.

::Time.zone = "Pacific"
^^^^^^^^^^^^^^^^^^^^^^^ Rails/TimeZoneAssignment: Use `Time.use_zone` with block instead of `Time.zone=`.
timezone, Time.zone = Time.zone, "Pacific Time (US & Canada)"
          ^ Rails/TimeZoneAssignment: Use `Time.use_zone` with block instead of `Time.zone=`.

timezone, Time.zone = Time.zone, "Pacific Time (US & Canada)"
          ^ Rails/TimeZoneAssignment: Use `Time.use_zone` with block instead of `Time.zone=`.

Time.zone, ENV['TZ'] = backed_up_zone, backed_up_tzvar
^ Rails/TimeZoneAssignment: Use `Time.use_zone` with block instead of `Time.zone=`.
