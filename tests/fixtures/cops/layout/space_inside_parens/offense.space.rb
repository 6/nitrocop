# nitrocop-config: EnforcedStyle: space
case p
in Point(*, 1, *a)
         ^ Layout/SpaceInsideParens: No space inside parentheses detected.
                 ^ Layout/SpaceInsideParens: No space inside parentheses detected.
  a
end

abort("hello
      ^ Layout/SpaceInsideParens: No space inside parentheses detected.
line two")
         ^ Layout/SpaceInsideParens: No space inside parentheses detected.
