# nitrocop-expect: 4:3 Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
# nitrocop-expect: 9:9 Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
foo(
  1,
  2,
  3
)

bar(
  "hello",
  "world"
)

some_method(
  foo,
  a: 1, b: 2
            ^ Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
)

some_method(
  a: 1, b: 2,
  c: 3,
      ^ Style/TrailingCommaInArguments: Avoid comma after the last parameter of a method call, unless each item is on its own line.
)
