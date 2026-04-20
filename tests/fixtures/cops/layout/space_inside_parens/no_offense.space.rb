# nitrocop-config: EnforcedStyle: space
case p
in Point( *, 1, *a )
  a
end

# No offense: line-continuation `\<newline>` inside a double-quoted string
# collapses into a single `tSTRING` token, so RuboCop's close-side check is
# skipped (the string token's line != `)` line).
abort( "hello \
world" )
