# nitrocop-config: EnforcedStyle: new_line

# Closing brace on separate line — OK under new_line
foo(a,
  b
)

# Block argument with closing brace on next line — OK
foo(a,
  &:to_s
)

# Block argument as the only argument with closing brace on next line — OK
map(
  &:to_s
)

# Single line — OK
foo(a, b)
