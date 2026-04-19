def foo(bar)
  puts bar
  x = bar + 1
  x
end

def baz(x)
  x
end

def qux(name)
  result = name.upcase
  result
end

# Reassignment that references the argument on the RHS is OK
def transform(name)
  name = name.to_s.strip
  name
end

def update(value)
  value = value + 1
  value
end

# Shorthand assignments always reference the arg
def increment(count)
  count += 1
  count
end

# Assignment inside conditional -- not flagged (imprecise)
def maybe(name)
  if something?
    name = 'default'
  end
  name
end

# Argument used before reassignment
def use_first(arg)
  puts arg
  arg = 'new'
  arg
end

# Block argument with RHS reference
items.each do |item|
  item = item.to_s
  puts item
end

# Argument reassigned but never referenced after -- RuboCop requires referenced?
def unused_after_reassign(bar)
  bar = 42
end

def unused_after_reassign2(bar)
  bar = 42
  puts 'done'
end

# Assignment only inside conditional, no outside reassignment
def conditional_only(foo)
  if bar
    foo = 42
  end
  puts foo
end

# Assignment only inside block, no outside reassignment
def block_only_assign(foo)
  something { foo = 43 }
  puts foo
end

# Block local variable (;j) is not a real argument -- should not flag
numbers = [1, 2, 3]
numbers.each do |i; j|
  j = i * 2
  puts j
end

# Shorthand assignment in block context should not flag
def bar_shorthand(bar)
  bar = 'baz' if foo
  bar ||= {}
end

# FP fix: argument reassigned but never read as Ruby variable (backtick/xstring)
def concat(other)
  other = `convertToArray(other)`
  `self.concat(other)`
end

# FP fix: argument reassigned but only "used" in string literal, not as variable
def process(a)
  a = 2
  puts "a"
end

# FP fix: argument reassigned, never read afterward at all
def shadow_no_read(a, b, c, d)
  a = 123
  b &&= 123
  c += 123
  d ||= 123
end

# Multi-assignment where RHS references the param (not shadowing)
def transform(result, data)
  result, extra = result.split(",")
  [result, extra]
end

# &block param used before reassignment
def wrapper(m, &block)
  block.call if block
  block = -> { m }
  block
end

# Multi-assignment where param is never read after (no offense)
def ignore_multi(location)
  location, line = get_location
end

# Multi-write from bare super should not be flagged (super implicitly forwards args)
def add_index_options(table_name, column_name, name: nil, enabled: false, **options)
  result, status, enabled = super
  [result, status, enabled]
end

# FP fix: binding before reassignment implicitly references all local variables
def self.new_with_attributes(id:, preset_name:, **other)
  arguments = Hash[binding.local_variables.map{ [_1, binding.local_variable_get(_1)]}]
  arguments.delete(:arguments)
  other = arguments.delete(:other)
  new(other.merge(arguments))
end

# FP fix: assignment in case predicate is conditional (RuboCop treats case as conditional parent)
def serialize(value)
  case value = super
  when ::Time
    Value.new(value)
  else
    value
  end
end

# FP fix: case predicate assignment with non-super RHS
def cast_value(value)
  case value = compute(value)
  when Value
    value.__getobj__
  else
    value
  end
end

# FP fix: case predicate assignment in block context
test "SequenceSet[input]" do |input|
  case (input = data[:input])
  when nil
    raise
  when String
    input
  end
end

# FP fix: assignment in a non-modifier if predicate is conditional
def validate_each(record, attribute, value)
  if value = record.try(attribute)
    super
  end
end

# FP fix: block argument assignment in an if predicate is conditional
listing.nodes.each do |node|
  if node = listing.nodes.find_by(name: name)
    self.name = node.name.succ
  end
end

# FP fix: method argument assignment in an if predicate stays conditional
def clear_code(url)
  if url = find(code)
    purge(code, url)
  end
end

# FP fix: chained predicate assignments under `and` stay conditional
tabs.each do |tab|
  if win = current_window and
    tab = win.current_tab
    tab.html?
  end
end
