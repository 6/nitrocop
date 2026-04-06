def foo
  self.name = "bar"
end

def test
  self.class
end

def example
  bar
end

self == other

def setter
  self.value = 42
end

# self. is required when a local variable shadows the method name
def _insert_record(values, returning)
  primary_key = self.primary_key
  primary_key
end

def build_snapshot(account_id: nil)
  account_id: account_id || self.account_id
end

def computed_permissions
  permissions = self.class.everyone.permissions | self.permissions
  permissions
end

# self.reader is allowed when self.writer= (compound assignment) exists in same scope
def calculated_confidence
  self.score ||= 1
  ups = self.score + 1
  ups
end

def with_op_assign
  self.count += 1
  total = self.count * 2
  total
end

class CompoundAcrossMethods
  def writer
    self.value ||= 1
  end

  def reader
    self.value
  end
end

def after_block_op_assign
  1.times do
    self.count += 1
  end
  self.count
end

def after_block_param_shadow
  people.each do |person|
    person.name
  end

  self.person.name
end

module SearchFilters
  included do
    scope :for_cycle, ->(cycle) {
      where(id: cycle.id)
    }

    def selected_cycle
      self.cycle
    end
  end
end

after_initialize do
  on(:post_created) do |post, _options|
    post.id
  end

  add_model_callback(PostAction, :after_commit, on: :create) do
    self.post
  end
end

module Referables
  module ClassMethods
    def configure_referables
      self.referable_fields ||= []
    end
  end

  def parse_referables
    self.referable_fields
  end
end

# Ruby keywords - self required to avoid parsing as keyword
def test_keywords
  self.alias
  self.and
  self.break
  self.case
  self.else
  self.elsif
  self.false
  self.in
  self.next
  self.nil
  self.not
  self.or
  self.redo
  self.retry
  self.self
  self.then
  self.true
  self.undef
  self.when
  self.__FILE__
  self.__LINE__
  self.__ENCODING__
end

# Kernel methods - self required to avoid ambiguity with Kernel functions
def test_kernel_methods
  self.open("file.txt")
  self.eval("code")
  self.fail("error")
  self.format("%.2f", 3.14)
  self.puts("hello")
  self.print("world")
  self.sleep(1)
  self.exit(0)
  self.system("ls")
  self.spawn("cmd")
  self.warn("caution")
  self.abort("fatal")
  self.exec("ls")
  self.rand(10)
  self.gets
  self.select
  self.loop
  self.require("foo")
  self.require_relative("bar")
  self.load("baz")
  self.lambda
  self.proc
  self.catch(:tag)
  self.throw(:tag)
  self.binding
  self.caller
  self.trap("INT")
  self.p("debug")
  self.printf("fmt")
  self.sprintf("fmt")
  self.Array(something)
  self.Integer("42")
  self.Float("3.14")
  self.String(42)
  self.Hash(pairs)
  self.Complex(1, 2)
  self.Rational(1, 3)
end

class DeclarativeTest
  self.test "declarative explicit receiver" do
    assert true
  end
end

# Block parameter shadows method name - self is required for disambiguation
%w[draft preview moderation approved rejected].each do |state|
  self.state == state
  define_method "#{state}?" do
    self.state == state
  end
end

# define_method block param shadows method name
STATUSES.each do |status|
  define_method("is_#{status}?") do
    self.status == status
  end
end

# Block param shadows method in simple iteration
BLOCKED_OBJECT_TYPES.each_value do |object_type|
  define_method("#{object_type}?") { self.object_type == object_type }
end

# Uppercase method names - could be confused with constants
def test_uppercase_methods
  self.Foo
  self.CALL_NAMED(name, false, expr)
  self.MyMethod
end

# if-prescan: lvasgn inside block inside if makes variable visible in condition
# (matches RuboCop's on_if behavior)
class PacketItem
  def as_json
    config = {}
    if self.limits
      if self.limits.values
        config['limits'] ||= {}
        config['limits']['persistence_setting'] = self.limits.persistence_setting
        config['limits']['enabled'] = true if self.limits.enabled
        self.limits.values.each do |limits_set, limits_values|
          limits = {}
          limits['red_low'] = limits_values[0]
        end
      end
    end
    config
  end
end

# keyword param name shadows method in default value
def plan_event(execution_plan_id: self.execution_plan_id)
end

# Operator methods called with dot syntax — self is required
def test_operator_methods
  self.+(other)
  self.-(other)
  self.*(other)
  self./(other)
  self.==(other)
  self.<<(item)
  self.<=>(other)
  self.>>(other)
  self.&(other)
  self.|(other)
end

# explicit call syntax is allowed
def to_proc = proc { |value| self.(value) }

# rescue => self.method — self is required when a local variable shadows the method
def process
  foo = 1
  begin
    risky_operation
  rescue => self.foo
    p foo
  end
end

# rescue => self.keyword — self is required when method name is a keyword
rescue => self.retry
