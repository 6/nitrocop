# nitrocop-config: EnforcedStyle: semantic

# Predicate usage is functional for braces but still allowed with do-end.
next if sinks.find do |sink|
  sink.tainted_value == cookie.name ||
    sink.tainted_value == cookie.value
end

# Hash pair values are return-value-of-scope, not return-value-used.
render json: {
  ingredients_with_errors: items.map do |item|
    item.id
  end
}, status: 422

# A block passed as the last super argument is also return-value-of-scope.
def set(configuration_hash)
  super(configuration_hash.transform_keys { transform_key(_1) })
end
