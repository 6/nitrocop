# nitrocop-config: EnforcedStyle: same_line

# Closing brace on same line as last arg — OK
foo(
  a,
  b)

# Block argument on same line as closing brace — OK
foo(a,
  &:to_s)

# Single line — OK
foo(a, b)
