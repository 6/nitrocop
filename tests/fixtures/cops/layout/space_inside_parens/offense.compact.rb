# nitrocop-config: EnforcedStyle: compact
g( f( x ) )
         ^ Layout/SpaceInsideParens: Space inside parentheses detected.

g( ( 3 + 5 ) * x )
  ^ Layout/SpaceInsideParens: Space inside parentheses detected.

warning(%(
        ^ Layout/SpaceInsideParens: No space inside parentheses detected.
  hi
))
 ^ Layout/SpaceInsideParens: No space inside parentheses detected.

uri_parse( to_absolute( url, page.url ) ).scheme == "https"
                                       ^ Layout/SpaceInsideParens: Space inside parentheses detected.

case p
in Point(*, 1, *a)
         ^ Layout/SpaceInsideParens: No space inside parentheses detected.
                 ^ Layout/SpaceInsideParens: No space inside parentheses detected.
  a
end
