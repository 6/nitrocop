# nitrocop-config: EnforcedStyle: compact
# Simple arrays with spaces
[ a, b, c, d ]
[ :a, :b ]
[ 'foo' ]
# Nested - consecutive right brackets collapsed (no space before ]])
[ 1, [ 2, 3, 4 ], [ 5, 6, 7 ]]
# Nested - consecutive left brackets collapsed (no space after [[)
[[ 2, 3, [ 4 ]]]
# 4-dimensional
[[[[ boom ]]]]
# Empty brackets (handled by EnforcedStyleForEmptyBrackets)
[]
# Multiline with bracket on own line
[
  1, 2, 3
]
# Multiline nested accepted
array = [[ a ],
  [ b, c ]]
# Constant array pattern with spaces (compact)
case value
in ADT[ *head, tail ]
end
# Nested constant pattern (collapsed right)
case value
in ADT[ *head, ADT[ *headhead, tail ]]
end
# %w/%i/%W/%I arrays are NOT real bracket arrays — don't collapse
[ %w[foo bar], %w[baz qux] ]
[ %i[a b c], %i[d e f] ]
[
  %i[annotations annotations resource_id],
  %i[permissions permissions role_id],
  %i[role_memberships memberships member_id]
]
# Mixed: real array + %w — only left side is multi-dim
[[ 1, 2 ], %w[a b] ]
# Multiline with element reference — not adjacent on left side, end_ok on right
[
  foo.bar,
  baz.qux
]
