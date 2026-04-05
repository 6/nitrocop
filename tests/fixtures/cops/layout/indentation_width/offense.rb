class Foo
    x = 1
    ^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
end

def bar
 y = 2
 ^^^ Layout/IndentationWidth: Use 2 (not 1) spaces for indentation.
end

if true
      z = 3
      ^^^ Layout/IndentationWidth: Use 2 (not 6) spaces for indentation.
end

items.each do |item|
      process(item)
      ^^^ Layout/IndentationWidth: Use 2 (not 6) spaces for indentation.
end

case x
when 1
      do_something
      ^^^ Layout/IndentationWidth: Use 2 (not 6) spaces for indentation.
end

# Block on chained method — dot on new line, body should indent from dot
source.passive_relationships
      .where(account: Account.local)
      .in_batches do |follows|
  process(follows)
  ^^^ Layout/IndentationWidth: Use 2 (not -4) spaces for indentation.
end

# Another chained block — body indented from end, not dot
Post.includes(:comments)
  .where("stuff")
  .references(:comments)
  .scoping do
  posts = authors(:david)
  ^^^ Layout/IndentationWidth: Use 2 (not 0) spaces for indentation.
end

# Block on chained method — body wrong relative to both dot and end columns
source
  .in_batches do |batch|
      process(batch)
      ^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
end

# begin...end block with wrong indentation
begin
x = 1
^^^ Layout/IndentationWidth: Use 2 (not 0) spaces for indentation.
rescue => e
  puts e
end

begin
      require 'builder'
      ^^^ Layout/IndentationWidth: Use 2 (not 6) spaces for indentation.
end

begin
    do_something
    ^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
rescue StandardError
  handle
end

# Assignment context: body should be indented from `if` keyword, not `end`
result = if condition
  value_one
  ^^^ Layout/IndentationWidth: Use 2 (not -7) spaces for indentation.
else
  value_two
end

      stream = if scheduler
        Stream.new(scheduler)
        ^^^ Layout/IndentationWidth: Use 2 (not -7) spaces for indentation.
      else
        nil
      end

# else body with wrong indentation
if cond
  func1
else
 func2
 ^^^ Layout/IndentationWidth: Use 2 (not 1) spaces for indentation.
end

# elsif body with wrong indentation
if a1
  b1
elsif a2
 b2
 ^^^ Layout/IndentationWidth: Use 2 (not 1) spaces for indentation.
else
  c
end

# rescue body with wrong indentation
begin
  do_something
rescue SomeException
 handle_error
 ^^^ Layout/IndentationWidth: Use 2 (not 1) spaces for indentation.
end

# ensure body with wrong indentation
begin
  do_something
ensure
    cleanup
    ^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
end

# rescue in def body with wrong indentation
def my_func
  do_something
rescue StandardError
 handle_error
 ^^^ Layout/IndentationWidth: Use 2 (not 1) spaces for indentation.
end

# unless body with wrong indentation
unless cond
 func
 ^^^ Layout/IndentationWidth: Use 2 (not 1) spaces for indentation.
end

# for loop body with wrong indentation
for var in 1..10
 func
 ^^^ Layout/IndentationWidth: Use 2 (not 1) spaces for indentation.
end

# singleton class body with wrong indentation
class << self
    def foo
    ^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
    end
end

# Class/module bodies should check every member, not only the first one
module Outer
    class Inner
    ^^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
        def one
        ^^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
        end

        def two
        ^^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
        end
    end
end

# Class bodies still check access modifier indentation
class AccessControlled
    private
    ^^^^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.

  def helper
  end
end

# else in begin/rescue with wrong indentation
begin
  do_something
rescue StandardError
  handle
else
   success_action
   ^^^ Layout/IndentationWidth: Use 2 (not 3) spaces for indentation.
end

# Tab-indented code — when IndentationStyle is 'spaces' (default),
# tabs count as 1 character, triggering "Use 2 (not 1)" offenses.
def tab_method
	x = 1
	^^^ Layout/IndentationWidth: Use 2 (not 1) spaces for indentation.
end

if tab_cond
	do_something
	^^^ Layout/IndentationWidth: Use 2 (not 1) spaces for indentation.
end

module TabModule
	CONST = 1
	^^^ Layout/IndentationWidth: Use 2 (not 1) spaces for indentation.
end

# def body with rescue — body should still be checked for indentation
def method_with_rescue
    do_something
    ^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
rescue StandardError
  handle_error
end

# def body with ensure — body should still be checked for indentation
def method_with_ensure
    do_something
    ^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
ensure
  cleanup
end

# def with misaligned end — indentation checked from def line start
  def misaligned_end_method
      body_here
      ^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
end

# Lambda block body with wrong indentation
scope :verified, -> do
    where('stuff')
    ^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
end

# Super with block — body wrong indentation
def each_mutation(payload, options = {}, &block)
  super(payload, options) do |element|
      yield element
      ^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
  end
end

# Case else body checked from last when keyword, not else keyword
result = case resource_name
         when :a
           do_a
         when :b
           do_b
     else
       default_name
       ^^^ Layout/IndentationWidth: Use 2 (not -2) spaces for indentation.
     end

# Class with first member being access modifier with args at wrong indentation
class Manager
    private :load, :available
    ^^^ Layout/IndentationWidth: Use 2 (not 4) spaces for indentation.
    public :load, :available
end
