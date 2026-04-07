# nitrocop-config: EnforcedStyle: new_line

# Closing brace on same line as last arg — offense under new_line
foo(a,
  b)
   ^ Layout/MultilineMethodCallBraceLayout: Closing method call brace must be on the line after the last argument.

# Block argument (&:to_s) on same line as closing brace
foo(a,
  &:to_s)
        ^ Layout/MultilineMethodCallBraceLayout: Closing method call brace must be on the line after the last argument.
