# nitrocop-expect: 4:3 Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
# nitrocop-expect: 9:9 Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
# nitrocop-expect: 14:12 Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
foo(
  1,
  2,
  3
)

bar(
  "hello",
  "world"
)

foo(
  bar,
  a: 1, b: 2
)
