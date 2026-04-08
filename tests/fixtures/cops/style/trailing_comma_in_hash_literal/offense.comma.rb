# nitrocop-expect: 3:6 Style/TrailingCommaInHashLiteral: Put a comma after the last item of a multiline hash.
# nitrocop-expect: 8:12 Style/TrailingCommaInHashLiteral: Put a comma after the last item of a multiline hash.
hash = {
  a: 1,
  b: 2
}

other = {
  foo: "bar",
  baz: "qux"
}

# FN fix: single-line hash with trailing comma
# nitrocop-expect: 12:23 Style/TrailingCommaInHashLiteral: Avoid comma after the last item of a hash, unless each item is on its own line.
single_tc = {a: 1, b: 2, }

# FN fix: multiline hash with elements sharing a line and trailing comma
# nitrocop-expect: 16:21 Style/TrailingCommaInHashLiteral: Avoid comma after the last item of a hash, unless each item is on its own line.
multi_shared = {
  id: 1, name: "test",
}

# FN fix: multiline hash with closing bracket on same line as last element, with trailing comma
# nitrocop-expect: 21:6 Style/TrailingCommaInHashLiteral: Avoid comma after the last item of a hash, unless each item is on its own line.
inline_tc = { a: 1,
  b: 2, }
