# require_parentheses style (default)

# Method calls with receiver and args but no parens
foo.bar 1, 2
^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

obj.method "arg"
^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

x.send :message, "data"
^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

# Receiverless calls inside method defs are NOT macros
def foo
  test a, b
  ^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
end

# Safe navigation operator also flags
top&.test a, b
^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

# Multiline chained method calls — offense is at start of full expression
expect(described_class.new)
  .to match_array(y)
# nitrocop-expect: 19:0 Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

custom_fields
  .include? attribute
# nitrocop-expect: 22:0 Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

# Receiverless call nested as argument to another call in class body
# is NOT a macro (parent in AST is send, not a wrapper)
class MyClass
  foo bar :baz
      ^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
end

# Receiverless calls inside case/when in class body are NOT macros
# (case/when are not wrappers in RuboCop's in_macro_scope?)
class MyClass
  case type
  when :foo
    test a, b
    ^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
  end
end

# Receiverless calls inside while/until in class body are NOT macros
class MyClass
  while running
    process_item a
    ^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
  end
end

# Receiverless calls inside rescue in class body are NOT macros
# (rescue is not a wrapper in RuboCop's in_macro_scope?)
class MyClass
  begin
    test a, b
    ^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
  rescue
    handle_error a
    ^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
  end
end

# Receiverless calls inside ensure in class body are NOT macros
class MyClass
  begin
    test a, b
    ^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
  ensure
    cleanup a
    ^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
  end
end

# yield with args and no parens in method body
def each_item
  yield element
  ^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
end

# yield with multiple args
def traverse(tree, &block)
  tree.each do |item|
    yield item, tree
    ^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
  end
end

# Ordinary method-call blocks do not inherit macro scope when the block
# expression is nested inside assignment or chaining.
trip = Trip.new(%i[call]) { require "pry" }
                            ^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

projects = 3.times.map { create :project, submitted_by: user }
                         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

expect {
  raise subject
  ^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
}.to raise_error(subject.class, message)
# nitrocop-expect: 84:0 Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

# Ternary branches are macros only when the ternary expression itself is.
if condition ? (yes_wizard? "yes") : (yes_wizard? "no")
                ^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
                                      ^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
  puts "x"
end

scope :alive,       -> { where alive: true }
                         ^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

scope :unreachable, -> { where alive: false }
                         ^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

scope :global,      -> { where global: true }
                         ^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

scope :fixed, -> { where fixed: true }
                   ^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

scope :by_severity, -> { order order_by_severity }
                         ^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

scope :read,   -> { where read: true }
                    ^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

scope :unread, -> { where read: false }
                    ^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

scope :global, -> { where global: true }
                    ^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

# Block argument only (no regular args) in method body — NOT a macro
def run_test(&block)
  instance_eval &block
  ^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
end

# Block argument with receiver — flagged
obj.instance_eval &block
^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

# Rescue modifier breaks macro scope — receiverless calls are NOT macros
require "objspace" rescue nil
^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

class MyClass
  allow_ip! "::1/128" rescue nil
  ^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
end

# Flow-control nodes are not wrappers; their argument calls are NOT macros
class Server
  get "/x" do
    next send_file static_file if static_file
         ^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
  end
end

# Parallel assignment breaks macro scope for ordinary block bodies
describe "x" do
  planned, running = plan.sub_plans.partition { |sub| planned? sub }
                                                      ^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
end

describe "x" do
  _out, err = capture_subprocess_io do
    call_event "hello", event, globals: { firestore_client: firestore_client }
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
  end
end

# Receiverless calls inside string interpolation are NOT macros
# (interpolated string is not a wrapper in RuboCop's in_macro_scope?)
"text #{bar :baz}"
        ^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

# BEGIN {} is not a wrapper — receiverless calls inside are NOT macros
BEGIN {
  require 'ostruct'
  ^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
}

# case/in (pattern matching) is not a wrapper — calls inside are NOT macros
case foo
in { a: 1 }
  puts "a"
  ^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
end

# Operator assignment (+=) breaks macro scope for receiverless calls
describe "x" do
  count += process_item arg
           ^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
end

# Pure begin only preserves macro scope when the whole expression is.
value = begin
  require 'rubygems/specification'
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
end

logger ||= begin
  require 'active_support/tagged_logging' unless defined?(ActiveSupport::TaggedLogging)
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
end

# `or begin` is also a non-wrapper parent for the begin body.
ready || begin
  warn "Automatic creation of the sandbox app failed"
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
  exit 1
  ^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
end

# Interpolated x-strings are not macro wrappers.
%x{#{raise TypeError, "x"}}
     ^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.

# A pure `begin ... end` whose parent is a setter assignment must NOT propagate
# macro scope. RuboCop's in_macro_scope? rejects the surrounding setter as a
# wrapper, so receiverless calls in the begin's `if`/`else` branches still flag.
Foo.bar.baz = begin
  if (token = ENV.fetch('SECRET_KEY_BASE', nil)).present?
    token
  elsif Rails.env.production?
    raise 'You must set SECRET_KEY_BASE'
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
  else
    sf = Rails.root.join('tmp/secret_key_base')
    if File.exist?(sf)
      File.read(sf)
    else
      puts "=> Generating initial SECRET_KEY_BASE in #{sf}"
      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Use parentheses for method calls with arguments.
      token = SecureRandom.hex(30)
      File.write(sf, token)
      token
    end
  end
end
