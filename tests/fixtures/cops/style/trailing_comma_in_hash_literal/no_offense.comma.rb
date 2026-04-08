hash = {
  a: 1,
  b: 2,
}

# Single-line hashes don't need trailing comma
single = {a: 1, b: 2}

# Empty hash
empty = {}

# FP fix: multiline hash where elements share a line — no comma needed
multi_shared = {
  id: 1, name: "test"
}

# FP fix: multiline with closing bracket on same line as last element
inline_close = { a: 1,
  b: 2 }

# FP fix: three elements, first two share a line
three_shared = {
  a: 1, b: 2,
  c: 3
}
