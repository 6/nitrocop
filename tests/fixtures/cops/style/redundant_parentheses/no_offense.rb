x = (1 + 2)
z = (foo ? bar : baz)
w = (a || b) ? 1 : 2
result = method_call(arg)
arr = [1, 2, 3]
# Chained parens
x = (a && b).to_s
# Splat
foo(*args)
# do..end block in argument to unparenthesized method call — parens are required
# to prevent Ruby from binding the block to the outer method
scope :advisory_lock, (lambda do |column:|
  column
end)
scope :display_all, (lambda do |after_id: nil|
  where(id: after_id)
end)
has_many :items, (proc do
  order(:position)
end)
# break/return/next with adjacent parens — keyword directly touching open paren
break(value) unless value
return(result) if done
next(item) if skip
# do..end blocks in hash values — parens prevent block binding to outer method
foo(default: (lambda do |routes|
  routes
end))
bar(key: (proc do
  something
end))
# Assignment in boolean context — parens disambiguate = from ==
(results[:dump_called] = true) && "dump_something"
(results[:load_called] = true) && "load_something"
x = (y = 1) && z
(a = foo) || bar
# Comparison inside another expression — not top-level, not flagged
x = (a == b) ? 1 : 2
result = (a > b) && c
# Comparison inside method body (return value) — has parent, not flagged
def edited?
  (last_edited_at - created_at > 1.minute)
end
# Comparison as hash value — has parent, not flagged
config = { enable_starttls: (ENV["VAR"] == "true") }
# Range literals — parens around ranges are almost never redundant
arr = [(1..5)]
ranges + [(line..line)]
(minimum..maximum).cover?(count)
x = (0..10)
# not/while/until — plausible (RuboCop doesn't flag these)
(not x)
(a until b)
(a while b)
# Operator with unparenthesized call arg
foo + (bar baz)
# Negative numeric base in exponentiation
(-2)**2
(-2.1)**2
# Unary on method call starting with integer literal
-(1.foo)
+(1.foo)
-(1.foo.bar)
+(1.foo.bar)
# Splat/kwsplat with operator expression
foo(*(bar & baz))
foo(*(bar + baz))
foo(**(bar + baz))
# Assignment in conditional — parens disambiguate = from ==
if (var = 42); end
unless (var = 42); end
while (var = 42); end
until (var = 42); end
# Unparenthesized method call with args used in boolean expression
(a 1, 2) && (1 + 1)
# rescue in method arg
foo((bar rescue baz))
# Multiple expressions in non-begin parent
x = (foo; bar)
x += (foo; bar)
x + (foo; bar)
x((foo; bar))
# Empty parens
()
# Chained unary
(!x).y
# Comparison in non-top-level context
x && (y == z)
(x == y).zero?
# Match regex against parenthesized expression
/regexp/ =~ (b || c)
regexp =~ (b || c)
# Command literal on RHS of regex match
unless /Version/m =~ (`convert -version`)
end
# rescue expression is plausible in certain contexts
foo((bar rescue baz))
# Parens around one-line rescue in array/hash/ternary
[(foo rescue bar)]
{ key: (foo rescue bar) }
cond ? (foo rescue bar) : 42
# post-condition loops with adjacent parens
begin
  do_something
end while(bar)
begin
  do_something
end until(bar)
# Parens touching keyword
if x; y else(1) end
if x; y else (1)end
# rescue keyword parens
begin
  some_method
rescue(StandardError)
end
# when keyword parens
case foo
when(Const)
  bar
end
# super/yield with hash arg
super ({
  foo: bar,
})
yield ({
  foo: bar,
})
# super/yield with multiline style argument
super (
  42
)
yield (
  42
)
# return with multiline style argument
return (
  42
)
# Unary operation (!x) in a chained boolean context — parens required for syntax
foo && (!x arg)
foo && (!x.m arg)
foo && (!super arg)
foo && (!yield arg)
foo && (!defined? arg)
# Unary operation (!x) when not sole expression and would change semantics
(!x arg) && foo
(!x.m arg) && foo
(!super arg) && foo
(!yield arg) && foo
(!defined? arg) && foo
# Non-parenthesized call with block — parens act as method arg grouping
method (:arg) { blah }
# method argument parentheses
method (arg)
# Keyword-form logical in parent context (and/or)
(1 and 2) and (3 or 4)
(1 or 2) or (3 and 4)
var = (foo or bar)
var = (foo and bar)
# Arithmetic operator parent for logical
x - (y || z)
# Hash literal as first arg of unparenthesized call — parens prevent { being parsed as block
x ({ y: 1 }), z
x ({ y: 1 }).merge({ y: 2 }), z
x ({ y: 1 }.merge({ y: 2 })), z
# Chained receiver with operator/logical inner — parens needed for operator precedence
(a + b).to_s(base)
(arr || []).each { |x| x }
(hash || {}).merge(other)
(a & b).include?(item)
# Safe-navigation bracket call in boolean context — RuboCop doesn't treat `&.[]` like plain `[]`
(@extensions&.[] name.to_sym) || false
# Assignment in default parameter value — parens syntactically required
def method(value = (not_set = true))
end
def suffix(value = (not_set = true; value))
end
def prompt(value = (default = "yes"))
end
def foo(bar = (baz = :quux))
end
# Assignment in default keyword parameter value
def method(key: (default = compute_default))
end
# Assignment used as the RHS of another assignment is still plausible
index = (state[:index] = (state.fetch :index, 0) + 1)
# Assignment used as the receiver of an index-or-write is still plausible
(icon_data = AdmonitionIcons[key] || {})[:name] ||= AdmonitionIcons[:note][:name]
# class << with assignment expression — parens needed to group assignment
class << (RANDOM = Random.new)
end
# def with assignment in receiver — parens needed
def (@matcher = BasicObject.new).===(obj)
  obj
end
# def with variable receiver — parens required for singleton method definition
def (@document).unwrap(obj); obj; end
def (@doc).version=(v); @version = v; end
# def with literal receiver — parens required
def (false).foo; end
# Block argument with assignment — parens required in &(expr)
m(&(l = -> {}))
m(1, &(callback = proc { |x| x }))
# Inline rescue with assignment — parens required
(t = File.mtime(target)) rescue nil
# Multiple assignment with assignment — parens required
a, *b, c = (e = [1, 2, 3, 4])
# Negative float in exponentiation with space
(-8.0) ** (1.0/3)
# Unary ! around non-method-call (assignment parens)
x = (!(found = find_file(exe)))
# Pattern matching in method argument — parens required
foo((bar in baz))
obj&.foo((bar in baz))
# Pattern matching in boolean operator — parens required
(foo in bar) && (baz in qux)
(foo in bar) || (baz in qux)
# Pattern matching in assignment — parens required
foo = (bar in baz)
foo ||= (bar in baz)
# Pattern matching in endless method definition — parens required
def myfoo = (bar in [0, 1])
def mybar = (baz => result)
# Assignment in endless method definition — parens required
def initialize(*values) = (@values = values)
# Unary on local variable — RuboCop's method_call_with_redundant_parentheses?
# returns false for non-call types (local variables), so these are not flagged.
# `x` is defined above in earlier fixture lines.
(!x)
(~x)
(-x)
(+x)
# Unary on variable — parens needed for operator precedence
# RuboCop's check_unary unwraps to the base; method_call_with_redundant_parentheses?
# returns false for variables (not :call type), so (-var) is not flagged.
num = 1
(-num) % 4
pad = (-num) % 4
# Unary plus on negated variable — +(-v)
v = 1
v.frozen? ? v : +(-v)
# Double negation — inner parens wrap a unary op, base is a paren node
!(!hash_of_options[:distinct])
!(!@values[:recursive])
# Unary minus on variable as method argument
x = 1
assert_tensor [-10, -20, -30], (-x)
# Multiline chained logical expression
xx = (organisations_string || "")
  .split(",")
  .collect(&:strip)
# Multiline chained method call
rounded_value =
  (fractional / smallest_denomination)
  .round(0, rounding_mode)
# Multiline chained nil-safe call
(sources || [])
  .grep(Source)
  .flat_map { _1 }
# Multiline chained with alignment
opt = (mailer.default_url_options || {})
      .merge(options)
      .merge(opt)
# Multiline chained comparison
(aa == bb)
  .to_s
# Non-empty while body — RuboCop only flags the empty-body form
while (pop_messages(queue_url, 10).length > 0)
  work
end
