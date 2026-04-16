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
