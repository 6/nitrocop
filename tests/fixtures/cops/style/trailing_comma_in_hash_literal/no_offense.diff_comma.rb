# nitrocop-config: EnforcedStyle: diff_comma
# Corpus FP: `diff_comma` accepts a trailing comma when the last item is
# followed by an immediate newline.
codeblock_delimiters = {
  '{'     => '}',
  'begin' => 'end',
  'do'    => 'end',
}

Apipie::ParamDescription.new(
  method_description,
  name,
  Numeric,
  {
    in: "path",
    required: true,
    added_from_path: true,
  }
)

hash = {
  a: 1, b: 2,
}
