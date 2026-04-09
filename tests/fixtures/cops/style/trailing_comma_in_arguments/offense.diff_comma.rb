# nitrocop-expect: 4:3 Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
# nitrocop-expect: 7:11 Style/TrailingCommaInArguments: Avoid comma after the last parameter of a method call, unless that item immediately precedes a newline.
# nitrocop-expect: 10:10 Style/TrailingCommaInArguments: Avoid comma after the last parameter of a method call, unless that item immediately precedes a newline.
foo(
  a,
  b,
  c
)

foo(a, b, c,)

foo(a: "b",
    c: "d",)
