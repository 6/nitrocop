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
# Multiline with %w[] elements (NOT multi-dimensional -- %w is not a bracket array)
[
  %w[foo bar],
  %w[baz qux]
]
# Single-line with %w[] elements
[ %w[foo bar], %w[baz qux] ]
# Multiline with %i[] elements
[
  %i[foo bar],
  %i[baz qux]
]
# Mixed %w[] and regular arrays
[ %w[a b], [ 1, 2 ]]
