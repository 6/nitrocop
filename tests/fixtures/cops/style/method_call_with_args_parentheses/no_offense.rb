# require_parentheses style (default)

# Method calls with parentheses
foo.bar(1, 2)

# No args — not checked
foo.bar

# Operators are exempt
x = 1 + 2

# Setter methods are exempt
foo.bar = baz

# Macros in class body (IgnoreMacros: true by default)
class MyClass
  include Comparable
  extend ActiveSupport
  prepend Enumerable
  attr_reader :name
  belongs_to :user
  has_many :posts
  validates :name, presence: true
  before_action :check_auth
end

# Macros in module body
module MyModule
  include Comparable
  extend ActiveSupport
end

# Top-level receiverless calls are macros too
puts "hello"
require "json"
raise ArgumentError, "bad"
p "debug"
pp object

# Macros inside blocks in class body
class MyClass
  concern do
    bar :baz
  end
end

# Macros inside begin in class body
class MyClass
  begin
    bar :baz
  end
end

# Macros in singleton class
class MyClass
  class << self
    bar :baz
  end
end

# super call with parens (super is not a CallNode)
def foo
  super(a)
end

# Macros inside Class.new do ... end (class constructor)
Class.new do
  include Comparable
  extend ActiveSupport
  attr_reader :name
end

# Macros inside Module.new do ... end
Module.new do
  include Comparable
  extend ActiveSupport
end

# Macros inside Struct.new do ... end
Struct.new(:x, :y) do
  include Comparable
end

# Class.new inside a method body — still class-like scope
def build_class
  Class.new do
    include Comparable
    attr_reader :name
  end
end

# Nested block inside Class.new
Class.new(Base) do
  concern do
    bar :baz
  end
end

# Class constructors act like class-like parents for receiverless arguments
module MyFormatters
  Widget = Class.new(remove_const :Widget) do
  end
end

# Class.new with block in if inside class (wrapper chain)
module MyMod
  if condition
    Class.new do
      include SomeThing
    end
  end
end

# Macros inside lambda inside block inside class (RuboCop macro? = true)
class MyController
  subject { -> { get :index } }
end

# Nested DSL blocks at the top level still count as macro scope
describe "x" do
  it "y" do
    create :project
  end
end

# Ternary branches in class body still count as macro scope
class UsersController < ApplicationController
  respond_to?(:before_action) ? (before_action :authenticate_user!) : (before_filter :authenticate_user!)
end

# yield with parentheses is fine in require_parentheses mode
def each_item
  yield(element)
end

# yield with no arguments is fine
def run
  yield
end

# omit_parentheses: calls inside when clauses are allowed (not flagged)
# nitrocop-config: EnforcedStyle: omit_parentheses
case type
when :comment
  comments_path(group_by: "comment", search: { ip_addr: ip_addr })
when :blip
  blips_path(search: { ip_addr: ip_addr })
end

# omit_parentheses: parentheses that only wrap an ambiguous descendant are allowed
def languages
  Array(foo((bar || [])))
end
