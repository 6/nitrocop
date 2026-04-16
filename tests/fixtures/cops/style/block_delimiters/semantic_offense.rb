def matches?(value)
  items.any? { |item| item.valid? }
             ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for procedural blocks.
rescue StandardError
  false
end

result = (items.map do |item|
                    ^^ Style/BlockDelimiters: Prefer `{...}` over `do...end` for functional blocks.
  item
end).join(", ")
