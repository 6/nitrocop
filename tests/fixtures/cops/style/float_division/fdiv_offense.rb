# nitrocop-config: EnforcedStyle: fdiv
# These SHOULD be flagged for fdiv style

# to_f on left (should be offense)
    a.to_f / b
    ^^^^^^^ Style/FloatDivision: Prefer using `fdiv` for float divisions.
    a.to_f / b.to_f
    ^^^^^^^^^^^^^ Style/FloatDivision: Prefer using `fdiv` for float divisions.
    a / b.to_f
    ^^^^^^^ Style/FloatDivision: Prefer using `fdiv` for float divisions.
