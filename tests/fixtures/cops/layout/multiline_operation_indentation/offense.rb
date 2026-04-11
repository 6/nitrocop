x = 1 +
        2
        ^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.
z = 5 +
      6
      ^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.
w = a &&
         b
         ^^^^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.

# Chained || with same-indent continuations (most common FN pattern)
def skip?
  a ||
  b ||
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
  c
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
end

# Multiline && in if condition - misaligned
if a &&
  b
  ^ Layout/MultilineOperationIndentation: Align the operands of a condition in an `if` statement spanning multiple lines.
  do_something
end

# FN: Assignment with chained + continuation at wrong indent
result = foo("h3") +
  foo("p1") +
  ^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.
  foo("p2")
  ^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.

# Same-indent chained + in assignment with wrong indent
result2 = "hello".capitalize +
  "world" +
  ^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.
  "foo"
  ^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.

# FN: Chained + at same indent in method body (no assignment context)
def lyrics
  "hello".capitalize +
  "world" +
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
  "foo"
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
end

# FN: Standalone chained + at column 0
"beer on the wall, ".capitalize +
"beer.\n" +
^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
"beer on the wall.\n"
^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
