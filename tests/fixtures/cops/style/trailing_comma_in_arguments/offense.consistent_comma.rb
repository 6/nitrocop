# nitrocop-expect: 4:3 Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
# nitrocop-expect: 9:9 Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
# nitrocop-expect: 12:11 Style/TrailingCommaInArguments: Avoid comma after the last parameter of a method call, unless items are split onto multiple lines.
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
