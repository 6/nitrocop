# nitrocop-config: EnforcedStyle: compact
# Simple array without spaces - missing on both sides
[a, b, c, d]
^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets missing.
           ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets missing.
# Missing space before close only
[ a, b, c, d]
            ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets missing.
# Missing space after open only
[a, b, c, d ]
^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets missing.
# Space between consecutive closing brackets (should collapse)
[ 1, [ 2, 3, 4 ], [ 5, 6, 7 ] ]
                              ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets detected.
# Space between consecutive opening brackets (should collapse)
[ [ a, b ], [ 1, 7 ]]
^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets detected.
# Both sides have space around consecutive brackets
[ [ a, b ], [ 1, 7 ] ]
                     ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets detected.
^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets detected.
# Constant array pattern without spaces
case value
in ADT[*head, tail]
      ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets missing.
                  ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets missing.
end
# Multiline nested arrays — brackets on different lines ARE collapsed in compact
[
^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets detected.
  [ :A, :D, false ],
  [ :N, :A, true ]
]
^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets detected.
[
^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets detected.
  [ 1, 2, 3 ],
  [ 4, 5, 6 ]
]
^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets detected.
# Element reference brackets count as adjacent (right side only)
[ foo[:bar] ]
            ^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets detected.
# Multiline with element reference — right side adjacent across lines
[
  attr[:name],
  attr[:type]
]
^ Layout/SpaceInsideArrayLiteralBrackets: Space inside array literal brackets detected.
