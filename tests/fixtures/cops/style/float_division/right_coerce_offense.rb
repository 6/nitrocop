# nitrocop-config: EnforcedStyle: right_coerce
# These SHOULD be flagged for right_coerce style

# to_f on left (should be offense)
    a.to_f / b
    ^^^^^^^^^^ Style/FloatDivision: Prefer using `.to_f` on the right side.
    a.to_f / b.to_f
    ^^^^^^^^^^^^^^^^^ Style/FloatDivision: Prefer using `.to_f` on the right side.
