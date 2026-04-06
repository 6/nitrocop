# nitrocop-config: EnforcedStyle: ternary
"#{condition ? 'foo' : ''}"

%(<nav #{
  data = []
  data.push(:x)
  data.join
}>)

%(<nav #{
  if keynav?
    'x'
  else
    'y'
  end
}>)

# Modifier if nested inside a method call — not a direct child of the
# interpolation, so RuboCop does not flag it.
"#{h(issue.status.name) + (" (#{format_date(issue.closed_on)})" if issue.closed?)}"

# Modifier unless nested inside a block — not a direct child either.
"#{items.map { |x| "#{x}" unless x.empty? }.join(', ')}"
