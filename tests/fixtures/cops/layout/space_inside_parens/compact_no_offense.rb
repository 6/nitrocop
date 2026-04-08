# nitrocop-config: EnforcedStyle: compact

# Correct compact form: spaces inside parens
f( 3 )
g = ( a + 3 )

# Empty parens: no space needed
y()

# Consecutive left parens: no space between them
g(( 3 + 5 ) * f )

# Consecutive right parens: no space between them
g( f( x ))
g( f( x( 3 )), 5 )

# Multiple consecutive right parens
a( b( c( d )))

# Multiple consecutive left parens
a((( x + 1 ) * 2 ))

# Two or more spaces between consecutive parens (RuboCop quirk: not flagged)
g( f( x )  )
g(  ( 3 + 5 ))
