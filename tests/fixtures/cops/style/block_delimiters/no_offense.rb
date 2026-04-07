# Single-line braces — correct
items.each { |x| puts x }

# Multi-line do..end — correct
items.map do |x|
  x * 2
end

[1, 2].each { |i| i + 1 }

items.select do |x|
  x > 0
end

3.times { |i| puts i }

# AllowedMethods: lambda (default)
scope :paginate, lambda { |limit, max_id = nil|
  query = order(arel_table[:id].desc).limit(limit)
  query = query.where(arel_table[:id].lt(max_id)) if max_id.present?
  query
}

# AllowedMethods: proc (default)
handler = proc { |x|
  x * 2
}

# AllowedMethods: it (default)
it { is_expected.to eq(true) }
it {
  is_expected.to eq(true)
}

# Non-parenthesized argument block — ignored (changing delimiters changes binding)
expect { subject }.to change {
  redis.zrange(key, 0, -1)
}.from([]).to(["foo"])

# Non-parenthesized keyword hash value containing a block
get '/:path', to: redirect { |params|
  "/#{params[:path]}"
}

# Single-line do-end with rescue clause — cannot convert to braces
foo do next unless bar; rescue StandardError; end

# Nested blocks inside non-parenthesized argument — all ignored
text html {
  body {
    input(type: 'text')
  }
}

# Deeply nested blocks inside non-parenthesized argument — all ignored
foo browser {
  text html {
    body {
      input(type: 'text')
    }
  }
}

# Chained multi-line brace blocks — inner blocks suppressed by outermost offense
# (see inline tests for chain behavior: offense_only_outermost_in_chain)

# Block inside operator method argument (non-parenthesized) — ignored
# because changing delimiters would change binding
"prefix" + items.map { |x|
  x.to_s
}.join(", ")

# Block chained inside << operator argument
result << items.collect { |item|
  item.name
}.join(", ")

# Lambda in non-parenthesized keyword arg — block inside lambda is suppressed
render node: -> {
  items.map { |item|
    item.name
  }
} do
  something
end

# super with single-line braces — correct
super(arg) { do_something }

# super with multi-line do-end — correct
super(arg) do
  do_something
end

# forwarding super with multi-line do-end — correct
super do
  yield if block_given?
  process
end

# Block inside assignment inside lambda body in keyword arg — suppressed
render node: -> {
  result = items.find { |item|
    item.name == "test"
  }
  unless result.nil?
    process(result)
  end
} do
  puts "rendered"
end

# Block using `it` (itblock) inside operator argument chain — ignored
# With TargetRubyVersion 4.0, RuboCop parses `it` blocks as :itblock,
# and `single_argument_operator_method?` returns false for itblocks because
# `block_type?` only matches :block. So `get_blocks` runs and ignores them.
spec_files = Dir.glob("lib/**/*.rb") +
             Dir.glob(
               "templates/**/*",
               flags: File::FNM_DOTMATCH
             ).reject {
               it =~ /\/\.{1,2}$/
             }.reject {
               it =~ /\.DS_Store$/
             } +
             [
               "exe/brut",
             ]

# Hash[] with multi-line chained block — allowed
Hash[list.map { |k, v|
  [k, v.to_s]
}.sort_by(&:first)]

# Multi-line brace block chained with compact
Hash[info.map { |k, v|
  [k, v]
}.compact]
