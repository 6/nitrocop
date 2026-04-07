# nitrocop-config: EnforcedStyle: grouped

class Foo
  include Bar
  ^^^^^^^^^^^ Style/MixinGrouping: Put `include` mixins in a single statement.
  include Qux
  ^^^^^^^^^^^ Style/MixinGrouping: Put `include` mixins in a single statement.
end

class Baz
  extend A
  ^^^^^^^^ Style/MixinGrouping: Put `extend` mixins in a single statement.
  extend B
  ^^^^^^^^ Style/MixinGrouping: Put `extend` mixins in a single statement.
end

class Quux
  prepend X
  ^^^^^^^^^ Style/MixinGrouping: Put `prepend` mixins in a single statement.
  prepend Y
  ^^^^^^^^^ Style/MixinGrouping: Put `prepend` mixins in a single statement.
  prepend Z
  ^^^^^^^^^ Style/MixinGrouping: Put `prepend` mixins in a single statement.
end