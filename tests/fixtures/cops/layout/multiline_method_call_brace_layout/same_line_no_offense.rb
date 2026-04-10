# nitrocop-config: EnforcedStyle: same_line

# Closing brace on same line as last arg — OK
foo(
  a,
  b)

# Block argument on same line as closing brace — OK
foo(a,
  &:to_s)

# Block argument as the only argument on same line as closing brace — OK
map(
  &:to_s)

# Legacy `Layout:LineLength` disable comment suppresses all Layout cops in RuboCop
# including this one.
# rubocop:disable Layout:LineLength
allow(double).to receive(:get)
  .and_return(
    Responses::Success.new(
      JSON.parse('{"a":1}')
    )
  )
# rubocop:enable Layout:LineLength

# Single line — OK
foo(a, b)
