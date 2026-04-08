# nitrocop-config: EnforcedStyle: compact

# Missing space: both sides
f(3)
  ^ Layout/SpaceInsideParens: No space inside parentheses detected.
   ^ Layout/SpaceInsideParens: No space inside parentheses detected.

# Missing space: both sides on grouping parens
g = (a + 3)
     ^ Layout/SpaceInsideParens: No space inside parentheses detected.
          ^ Layout/SpaceInsideParens: No space inside parentheses detected.

# Empty parens with space
y( )
  ^ Layout/SpaceInsideParens: Space inside parentheses detected.

# Extraneous space between consecutive right parens
g( f( x ) )
         ^ Layout/SpaceInsideParens: Space inside parentheses detected.

# Extraneous space between consecutive right parens (nested call)
g( f( x( 3 ) ), 5 )
            ^ Layout/SpaceInsideParens: Space inside parentheses detected.

# Extraneous space between consecutive left parens, plus missing close space
g( ( ( 3 + 5 ) * f) ** x, 5 )
  ^ Layout/SpaceInsideParens: Space inside parentheses detected.
    ^ Layout/SpaceInsideParens: Space inside parentheses detected.
                  ^ Layout/SpaceInsideParens: No space inside parentheses detected.

# def with missing space
def configure(options)
              ^ Layout/SpaceInsideParens: No space inside parentheses detected.
                     ^ Layout/SpaceInsideParens: No space inside parentheses detected.
end
