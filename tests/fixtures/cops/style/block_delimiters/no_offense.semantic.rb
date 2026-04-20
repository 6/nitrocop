# nitrocop-config: EnforcedStyle: semantic

def set(configuration_hash)
  super(configuration_hash.transform_keys { transform_key(_1) })
end

payload = {
  ingredients_with_errors: items.map do |item|
    { id: item.id }
  end
}

if sinks.find do |sink|
     sink.tainted_value == cookie.name ||
       sink.tainted_value == cookie.value
   end
  process
end

def selected_ids(items)
  return items.map { |item|
    item.id
  }
end

defined?(not_a_method { 1 }).should == "expression"

def log_error
  do_work
rescue => e
  logger.debug { e.message }
end

assert_equal(*args.map { |str|
  str.tr("\n", "")
})

yield records.filter_map { |record| record.instance_variable_get(records_variable_name) }

# Earlier blocks structurally equal to the last child inherit rv_of_scope in RuboCop
def cached
  assert_queries(1) { assert_equal("foo", fetch(1)) }

  update!

  assert_queries(1) { assert_equal("foo", fetch(1)) }
end

def reverse_each_examples
  ARRAY.reverse.each { |x| x }

  ARRAY.reverse.each do |x|
    x
  end
end

ActiveRecord::Base.cache do
  controller.send(:in_paginated_batches, &Proc.new {})
end

def build_dest(rels, ret)
  dest = rels.inject(ret) { |h, rel| h[rel] ||= {} }["columns"] ||= []
  dest
end

parsers.each do |parser|
  return backtrack { parser.call }
rescue Error
  next
end

options.reduce([""]) do |s, n|
  next Array(s).map { |a| a + n } if n.is_a?(String)

  Array(s).product(n.every).map(&:join)
end

def initialize(hooks = Hash.new { |h, k| h[k] = [] })
  @hooks = hooks
end

it "passes coerced value if it doesn't meet constraints" do
  called = false

  type.("15") do |coerced|
    called = true
    expect(coerced).to be(15)
  end

  expect(called).to be(true)
end

helper = 1
map { _1**2 }
