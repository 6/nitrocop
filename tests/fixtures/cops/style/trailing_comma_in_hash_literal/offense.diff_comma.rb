# nitrocop-expect: 3:6 Style/TrailingCommaInHashLiteral: Put a comma after the last item of a multiline hash.
# nitrocop-expect: 13:25 Style/TrailingCommaInHashLiteral: Avoid comma after the last item of a hash, unless that item immediately precedes a newline.
hash = {
  a: 1,
  b: 2
}

Apipie::ParamDescription.new(
  method_description,
  name,
  Numeric,
  {
    in: "path",
    required: true,
    added_from_path: true, }
)
