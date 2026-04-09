# Nested hashes collapse successive right braces
conn.get(path, { page: { size: 100, number: page }})
# Hash key/value literals collapse successive left and right braces
foo = {{ a: 1 } => { b: { c: 2 }}}
