# nitrocop-config: EnforcedStyle: compact
g( ( x ))
  ^ Layout/SpaceInsideParens: Space inside parentheses detected.
g( f( x ) )
         ^ Layout/SpaceInsideParens: Space inside parentheses detected.
case p
  in Point(*, 1,*a)
           ^ Layout/SpaceInsideParens: No space inside parentheses detected.
                  ^ Layout/SpaceInsideParens: No space inside parentheses detected.
    a
end
