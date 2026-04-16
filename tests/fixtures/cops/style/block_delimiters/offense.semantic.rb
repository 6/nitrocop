# nitrocop-config: EnforcedStyle: semantic

def matches?(value)
  klasses.any? { |klass| value.is_a?(klass) }
               ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for procedural blocks.
rescue NoMethodError
  false
end
