# Different collections
items.each { |item| do_something(item) }
other_items.each { |item| do_something(item) }

# Interleaved with code
items.each { |item| foo(item) }
do_something
items.each { |item| bar(item) }

# Different loop methods on same collection
items.reverse_each { |item| do_something(item) }
items.each { |item| do_something(item) }

# Same method with different arguments (e.g., each_slice)
each_slice(2) { |slice| do_something(slice) }
each_slice(3) { |slice| do_something(slice) }

# Different receiver with safe navigation
foo(:bar)&.each { |item| do_something(item) }
foo(:baz)&.each { |item| do_something(item) }

# Empty loops — both bodies empty
items.each {}
items.each {}

# for loops over different collections
for item in items do do_something(item) end
for foo in foos do do_something(foo) end

# for loops interleaved with code
for item in items do do_something(item) end
some_code
for item in items do do_something_else(item, arg) end

# Non-loop methods — map, select, reject are NOT looping methods per RuboCop
items.map { |item| do_something(item) }
items.map { |item| do_something_else(item) }

items.select { |item| item.valid? }
items.select { |item| item.active? }

items.reject { |item| item.nil? }
items.reject { |item| item.empty? }

items.collect { |item| item.to_s }
items.collect { |item| item.to_i }

items.flat_map { |item| item.children }
items.flat_map { |item| item.parents }

# Block argument (&:method) — not a block type per RuboCop
items.each(&:to_s)
items.each(&:to_i)

# Block argument followed by regular block
items.each(&:reload)
items.each { |item| do_something(item) }

# Regular block followed by block argument
other.each { |item| do_something(item) }
other.each(&:to_s)

# Explicit begin body
def test_inside_begin
  begin
    items.each { |item| do_something(item) }
    items.each { |item| do_something_else(item) }
  end
end

# FP fix: %w (string array) followed by %i (symbol array) — different receiver types
%w{automatic manual disabled}.each do |type|
  resource.startup_type type
  expect(resource.startup_type).to eql(type.to_sym)
end

%i{automatic manual disabled}.each do |type|
  resource.startup_type type
  expect(resource.startup_type).to eql(type)
end

# FP fix: boolean array followed by %w string array — different element types
[true, false].each { |bool| expect(converted?(bool)).to eq true }
%w(true false).each { |string| expect(converted?(string)).to eq false }

# Different heredoc receiver contents
<<END.split.each { |f| require "sequel/extensions/#{f}" }
date_arithmetic
END

<<END.split.each { |f| require "sequel/plugins/#{f}" }
association_dependencies
END
