# nitrocop-config: EnforcedStyle: semantic

# Last statement inside a begin/rescue body is still procedural.
def matches?(value)
  klasses.any? { |klass| value.is_a?(klass) }
               ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for procedural blocks.
rescue NoMethodError
  false
end

# Parenthesized receivers that are chained still use the block return value.
(@pages.map do |page|
            ^^ Style/BlockDelimiters: Prefer `{...}` over `do...end` for functional blocks.
  page
end).compact
