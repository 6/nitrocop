# nitrocop-config: EnforcedStyle: consistent_comma

# Multiline call missing trailing comma
foo(
  1,
  2,
  3
   ^ Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
)

# Single-line call with trailing comma — offense
bar(1, 2,)
        ^ Style/TrailingCommaInArguments: Avoid comma after the last parameter of a method call, unless items are split onto multiple lines.

# Index compound assignment — multiline should require comma
response_headers[
  key
     ^ Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
] ||= "value"

v[
  0
   ^ Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
] += 42

hash[
  :foo
      ^ Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
] &&= :bar
