class Foo
  x = 1
end

def bar
  y = 2
end

if true
  z = 3
end

while true
  a = 1
end

module Baz
  CONST = 1
end

def single_line; end

# Block body indented from line start, not from do/{
items.each do |item|
  process(item)
end

settings index: index_preset(refresh_interval: '30s') do
  field(:id, type: 'long')
end

[1, 2].map { |x|
  x * 2
}

case x
when 1
  do_something
when 2
  do_other
end

# Block on chained method — body indented relative to dot
source.passive_relationships
      .where(account: Account.local)
      .in_batches do |follows|
        process(follows)
      end

# Block body indented from dot when dot is on a new line (matching RuboCop)
source.passive_relationships
      .where(account: Account.local)
      .in_batches do |follows|
        process(follows)
      end

# Chained method with do..end block, body indented from dot
account.conversations
       .joins(:inbox)
       .where(created_at: range)
       .each_with_object({}) do |((channel_type, status), count), grouped|
         grouped[channel_type] ||= {}
         grouped[channel_type][status] = count
       end

# Block with dot NOT on a new line — uses end column as base
items.each do |item|
  process(item)
end

# Assignment context: body indented from `if` keyword column
x = if foo
      bar
    end

result = if condition
           value_a
         end

y = while queue.any?
      queue.pop
    end

z = until done
      process_next
    end

# Assignment context (keyword style): body indented from keyword, end at keyword
links = if enabled?
          body
        end

# Inline block wrapping — closing } on same line as body
get "/", constraints: lambda { |req|
  req.subdomain.present? && req.subdomain != "clients" },
           to: lambda { |env| [200, {}, %w{default}] }

# Block params on same line as body
files = (Dir["test/**/*_test.rb"].reject {
  |x| x.include?("/adapters/")
} + Dir["test/other/**/*_test.rb"]).sort

# Multi-line when with `then` on continuation line
case type
when :references, :belongs_to,
     :attachment, :attachments,
     :rich_text                   then nil
when :string
  "MyString"
end

# Misaligned end with body correctly indented from `if` keyword
# (EndAlignment disabled scenario — end at arbitrary column)
x = if foo
      bar
    end

# Misaligned end with body correctly indented from `while` keyword
y = while queue.any?
      queue.pop
    end

# Misaligned end with body correctly indented from `until` keyword
z = until done
      process_next
    end

# begin...end block with correct indentation
begin
  require 'builder'
rescue LoadError
  # skip
end

begin
  x = 1
  y = 2
end

# begin...end in assignment context — body indented from `end`, not `begin`
result = begin
  compute_value
rescue StandardError
  nil
end

@cache ||= begin
  load_cache
end

# else body correctly indented
if cond
  func1
else
  func2
end

# elsif body correctly indented
if a1
  b1
elsif a2
  b2
else
  c
end

# rescue body correctly indented
begin
  do_something
rescue StandardError
  handle_error
end

# ensure body correctly indented
begin
  do_something
ensure
  cleanup
end

# rescue in def correctly indented
def my_func
  do_something
rescue StandardError
  handle_error
end

# unless body correctly indented
unless cond
  func
end

# for loop body correctly indented
for var in 1..10
  func
end

# singleton class body correctly indented
class << self
  def foo
  end
end

# else in begin/rescue correctly indented
begin
  do_something
rescue StandardError
  handle
else
  success_action
end

# rescue after empty body (no offense)
begin
rescue
  handle_error
end

# ensure after empty body (no offense)
begin
ensure
  something
end

# rescue after empty def (no offense)
def foo
rescue
  handle_error
end

# Block body starting with access modifier — skip indentation check
# (matches RuboCop's starts_with_access_modifier? skip)
m = Module.new do
    module_function

  def some_method
    true
  end
end

# Class body starting with access modifier still accepts correctly aligned modifiers
class Foo
  private

  def bar
    baz
  end
end

# Body not at first char on line — skip (RuboCop skip_check?)
if cond then result = value
end

# Access modifier with symbol arg — RuboCop skips SUBSEQUENT access modifiers
# in class member walk (handled by Layout/AccessModifierIndentation).
# The FIRST member is always checked regardless.
class AccessModWithArgs
  private :some_method
    public :other_method
    protected :another_method
end

# Lambda block body correctly indented
scope :verified, -> do
  where('stuff')
end

# Super with block — body correctly indented
def each_mutation(payload, options = {}, &block)
  super(payload, options) do |element|
    yield element
  end
end

# Case else body correctly indented relative to last when keyword
result = case x
         when 1
           do_a
     else
           do_else
         end

# def body with rescue — correctly indented
def method_with_rescue
  do_something
rescue StandardError
  handle_error
end

# def body with ensure — correctly indented
def method_with_ensure
  do_something
ensure
  cleanup
end
