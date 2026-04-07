# nitrocop-config: EnforcedStyle: forbidden
# module_function with arguments should NOT be flagged (only bare module_function is forbidden)
module Foo
  def bar; end
  module_function :bar
end

# module_function with multiple arguments should NOT be flagged
module Baz
  def a; end
  def b; end
  module_function :a, :b
end