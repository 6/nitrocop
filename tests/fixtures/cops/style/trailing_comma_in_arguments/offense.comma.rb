# nitrocop-expect: 4:3 Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
# nitrocop-expect: 9:9 Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
# nitrocop-expect: 12:11 Style/TrailingCommaInArguments: Avoid comma after the last parameter of a method call, unless each item is on its own line.
# nitrocop-expect: 16:15 Style/TrailingCommaInArguments: Avoid comma after the last parameter of a method call, unless each item is on its own line.
foo(
  1,
  2,
  3
)

bar(
  "hello",
  "world"
)

foo(a, b, c,)

on('--scope', Integer,
    'How many',
    "(Default)",
)
