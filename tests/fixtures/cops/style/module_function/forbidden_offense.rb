# nitrocop-config: EnforcedStyle: forbidden
module Foo
  module_function
  ^^^^^^^^^^^^ Style/ModuleFunction: `module_function` and `extend self` are forbidden.
  def bar; end
end

module Bar
  extend self
  ^^^^^^^^^^^ Style/ModuleFunction: `module_function` and `extend self` are forbidden.
  def baz; end
end