# nitrocop-config: EnforcedStyle: grouped

# sclass body should not be checked for mixin grouping
# (RuboCop does not define on_sclass, only on_class and on_module)
class Foo
  prepend X
  class << self
    prepend Y
  end
end

class Bar
  include A
  class << self
    include B
  end
end

class Baz
  extend X
  class << self
    extend Y
  end
end