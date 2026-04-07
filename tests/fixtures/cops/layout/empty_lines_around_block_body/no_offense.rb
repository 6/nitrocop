items.each do |x|
  puts x
end

[1, 2, 3].each { |x| puts x }

[1, 2].map do |x|
  x * 2
end

# Backslash line continuation before do — not a blank body beginning
run_command(arg1, arg2) \
  do |channel, expected|

  process(channel, expected)
end

# Lambda brace block without blank lines
action = -> (a) {
  a.map { |c| c.name }
}

# Lambda do block without blank lines
handler = -> (opts = {}) do
  opts.each { |k, v| puts v }
end

# Lambda with multiline params and do — blank line after do is not flagged
# because RuboCop uses send_node.last_line (the -> line) as the reference
scope :_candlestick, -> (timeframe: '1h',
                   segment_by: 'symbol',
                   time: 'created_at',
                   volume: 'volume',
                   value: 'close') do

  select("something")
end

# Lambda with multiline params and brace — blank line after { is not flagged
transformer = -> (first:,
                  second:,
                  third: nil) {

  [first, second, third].compact
}

# nitrocop-config: EnforcedStyle: empty_lines
# Comment-only block body — empty bodies should not trigger empty_lines check
bar do
  # only a comment
end
