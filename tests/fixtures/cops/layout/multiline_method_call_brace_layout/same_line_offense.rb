# nitrocop-config: EnforcedStyle: same_line

# Closing brace on separate line — offense under same_line
foo(
  a,
  b
)
^ Layout/MultilineMethodCallBraceLayout: Closing method call brace must be on the same line as the last argument.

# Block argument (&:to_s) — closing brace on separate line
foo(a,
  &:to_s
)
^ Layout/MultilineMethodCallBraceLayout: Closing method call brace must be on the same line as the last argument.
