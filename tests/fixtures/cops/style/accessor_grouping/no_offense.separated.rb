# Single-arg accessors are fine in separated style
class Foo
  attr_reader :bar
  attr_reader :baz
end

# Multi-arg accessor with comment on previous line - not flagged
class WithComment
  # Some comment
  attr_reader :one, :two
end

# Multi-arg accessor with annotation on previous line - not flagged (not groupable)
class WithAnnotation
  annotation_method :bar
  attr_reader :one, :two
end

# Single accessor in class
class Single
  attr_reader :bar
end

# Different single-arg accessor types
class DifferentTypes
  attr_reader :a
  attr_writer :b
  attr_accessor :c
end
