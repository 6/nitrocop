# nitrocop-config: EnforcedStyle: diff_comma
some_method(
  a,
  b
   ^ Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
)

response_headers[
  HEADER
        ^ Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
] ||= "placeholder"

v = nil
v [
  0
   ^ Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
] += 42

current_standings[
  user_standing.user_id
                       ^ Style/TrailingCommaInArguments: Put a comma after the last parameter of a multiline method call.
] &&= fallback
