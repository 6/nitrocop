# Basic multi-arg accessor should be flagged
class Foo
  attr_reader :bar, :baz
  ^^^^^^^^^^^^^^^^^^^^^^ Style/AccessorGrouping: Use one attribute per `attr_reader`.
  attr_accessor :quux
  other_macro :zoo, :woo
end

# Multi-arg accessors with different access modifiers
class WithModifiers
  attr_reader :bar1, :bar2
  ^^^^^^^^^^^^^^^^^^^^^^^^ Style/AccessorGrouping: Use one attribute per `attr_reader`.

  protected
  attr_accessor :quux

  private
  attr_reader :baz1, :baz2
  ^^^^^^^^^^^^^^^^^^^^^^^^ Style/AccessorGrouping: Use one attribute per `attr_reader`.
  attr_writer :baz3
  attr_reader :baz4

  public
  attr_reader :bar3
  other_macro :zoo
end

# Multi-arg accessors within eigenclass
class WithEigenclass
  attr_reader :bar

  class << self
    attr_reader :baz1, :baz2
    ^^^^^^^^^^^^^^^^^^^^^^^^ Style/AccessorGrouping: Use one attribute per `attr_reader`.
    attr_reader :baz3

    private

    attr_reader :quux1, :quux2
    ^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/AccessorGrouping: Use one attribute per `attr_reader`.
  end
end

# Multi-arg accessor after other method + blank line is still flagged
class AfterOtherMethod
  other_macro :zoo, :woo

  attr_reader :foo, :bar
  ^^^^^^^^^^^^^^^^^^^^^^ Style/AccessorGrouping: Use one attribute per `attr_reader`.
end

# attr_writer and attr_accessor with multiple args
class MultipleTypes
  attr_writer :a, :b
  ^^^^^^^^^^^^^^^^^^ Style/AccessorGrouping: Use one attribute per `attr_writer`.
  attr_accessor :c, :d
  ^^^^^^^^^^^^^^^^^^^^ Style/AccessorGrouping: Use one attribute per `attr_accessor`.
end

# In a module
module MyMod
  attr_reader :x, :y
  ^^^^^^^^^^^^^^^^^^ Style/AccessorGrouping: Use one attribute per `attr_reader`.
end
