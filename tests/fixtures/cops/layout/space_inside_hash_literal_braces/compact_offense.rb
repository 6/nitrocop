# Nested hashes with a space between consecutive right braces
h = { a: { b: 2 } }
                 ^ Layout/SpaceInsideHashLiteralBraces: Space inside } detected.
# Hash key/value literals with a space between consecutive left braces
foo = { { a: 1 } => { b: { c: 2 }}}
       ^ Layout/SpaceInsideHashLiteralBraces: Space inside { detected.
