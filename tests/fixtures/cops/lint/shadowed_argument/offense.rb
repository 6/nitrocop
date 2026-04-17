def foo(bar)
  bar = 'something'
  ^^^^^^^^^^^^^^^^^ Lint/ShadowedArgument: Argument `bar` was shadowed by a local variable before it was used.
  bar
end

def baz(x, y)
  x = 42
  ^^^^^^ Lint/ShadowedArgument: Argument `x` was shadowed by a local variable before it was used.
  x + y
end

def qux(name)
  name = compute_name
  ^^^^^^^^^^^^^^^^^^^^ Lint/ShadowedArgument: Argument `name` was shadowed by a local variable before it was used.
  name
end

# Assignment inside conditional followed by unconditional reassignment
def cond_then_assign(foo)
                     ^^^ Lint/ShadowedArgument: Argument `foo` was shadowed by a local variable before it was used.
  if bar
    foo = 43
  end
  foo = 42
  puts foo
end

# Assignment inside block followed by unconditional reassignment
def block_then_assign(foo)
                      ^^^ Lint/ShadowedArgument: Argument `foo` was shadowed by a local variable before it was used.
  something { foo = 43 }
  foo = 42
  puts foo
end

# Assignment inside lambda followed by unconditional reassignment
def lambda_then_assign(foo)
                       ^^^ Lint/ShadowedArgument: Argument `foo` was shadowed by a local variable before it was used.
  lambda do
    foo = 43
  end
  foo = 42
  puts foo
end

# Block argument: conditional then unconditional reassignment
do_something do |foo|
                 ^^^ Lint/ShadowedArgument: Argument `foo` was shadowed by a local variable before it was used.
  if bar
    foo = 43
  end
  foo = 42
  puts foo
end

# FN fix: arg = super shadows the argument (super doesn't explicitly reference arg)
def deserialize(value)
  value = super
  ^^^^^^^^^^^^^ Lint/ShadowedArgument: Argument `value` was shadowed by a local variable before it was used.
  cast_value(value) unless value.nil?
end

# FN fix: arg = super in method with more params
def normalize_key(key, options)
  key = super
  ^^^^^^^^^^^ Lint/ShadowedArgument: Argument `key` was shadowed by a local variable before it was used.
  process(key)
end

# FN fix: block arg shadowed inside a method (nested block)
def process_data
  items.each do |v|
    v = transform(v.to_s)
    use(v)
  end
  og.data.each do |k, v|
    next if k == "title_attr"
    v = og.send(k)
    ^^^^^^^^^^^^^^ Lint/ShadowedArgument: Argument `v` was shadowed by a local variable before it was used.
    @raw[k] ||= v unless v.nil?
  end
end

# FN fix: block arg replaced with global variable
gsub(/pattern/) {|match|
  match = $1
  ^^^^^^^^^^ Lint/ShadowedArgument: Argument `match` was shadowed by a local variable before it was used.
  process(match)
}

# FN fix: multi-assignment shadows method arg
def search(url, args, options)
  option_item, args = options[:task], options[:args]
               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Lint/ShadowedArgument: Argument `args` was shadowed by a local variable before it was used.
  process(args)
end

# FN fix: multi-assignment shadows optional method arg
def check(result, location = nil)
  location, line = get_location(result)
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Lint/ShadowedArgument: Argument `location` was shadowed by a local variable before it was used.
  [location, line]
end

# FN fix: boolean expression contains assignment that shadows block param
def scrub(char=nil, &block)
  char && block = lambda { |c| char }
          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Lint/ShadowedArgument: Argument `block` was shadowed by a local variable before it was used.
  block.call("x")
end

# FN fix: &block param shadowed by simple assignment
def handle(m, *args, &block)
  block = Registry.matchers[m.to_sym]
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Lint/ShadowedArgument: Argument `block` was shadowed by a local variable before it was used.
  instance_exec(args.first, &block)
end

# FN fix: keyword-rest argument shadowed by local assignment
def regex_apply(text, pattern, flags = "", **options)
  flags = flags.to_s
  options = 0
  ^^^^^^^^^^^ Lint/ShadowedArgument: Argument `options` was shadowed by a local variable before it was used.
  options |= Regexp::IGNORECASE if flags.include?(?i)
  Regexp.new(pattern, options) === text
end

# FN fix: keyword-rest argument shadowed by local assignment (second corpus shape)
def replace_apply(text, pattern, replacement, flags = "", **options)
  flags = flags.to_s
  options = 0
  ^^^^^^^^^^^ Lint/ShadowedArgument: Argument `options` was shadowed by a local variable before it was used.
  options |= Regexp::MULTILINE if flags.include?(?m)
  text.to_s.gsub(Regexp.new(pattern, options), replacement)
end

# FN fix: optional argument shadowed conditionally, then reassigned unconditionally
def page(page = nil, options = {})
                     ^^^^^^^^^^^^ Lint/ShadowedArgument: Argument `options` was shadowed by a local variable before it was used.
  options, page = page, nil if page.is_a? Hash
  page_param = pager_option(:page_param, options)
  options.delete(page_param)
  query = options.dup
  collection = scoped_query(options = { limit: 1 }.merge(query))
  collection
end

# FN fix: optional argument shadowed in conditional before later unconditional multi-assignment
def initialize(x, y = nil, &action)
                  ^^^^^^^ Lint/ShadowedArgument: Argument `y` was shadowed by a local variable before it was used.
  if x.is_a? Hash
    y = x[:y]
    x = x[:x]
  end
  use_pair(x, y)
  _, x, y = check_anchor
  set_position(x, y)
  action&.call
end

# FN fix: def nested inside `begin/ensure` must not treat the outer begin body
# as a conditional branch for the def's own short-circuit shadowing.
begin
  def scrub(char = nil, &block)
    char && block = lambda { |c| char }
            ^^^^^^^^^^^^^^^^^^^^^^^^^^^ Lint/ShadowedArgument: Argument `block` was shadowed by a local variable before it was used.
    block.call("x")
  end
ensure
  nil
end
