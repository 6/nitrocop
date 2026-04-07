class Foo
end

module Bar
end

class Foo
  class Bar
  end
end

module FooModule
  module BarModule
  end
end

class FooClass
  class BarClass
  end
end

# Class inside class with inheritance (common pattern in policy objects)
class InboxPolicy < ApplicationPolicy
  class Scope
    def resolve
      super
    end
  end
end

# Module inside class (nested style is fine)
class MyService
  module Helpers
    def help; end
  end
end

# Single-name cbase — not flagged (::Foo has no namespace separator)
class ::Foo
end

module ::Bar
end

# Compact-style as sole body of outer module — not flagged
# (in RuboCop, node.parent is the module, so it's skipped)
module Wrapper
  class Inner::Name
  end
end

module Outer
  module Inner::Nested
  end
end

# Expression-based class/module definitions — RuboCop crashes on these
x = module Puppet::Parser::Functions
  1
end

@memory_class = class Testing::MyMemory < Puppet::Indirector::Memory
  self
end

# nitrocop-config: EnforcedStyle: compact,compact
# Compact-style module at offset 0 with no final newline — RuboCop crashes
# on autocorrect due to AlignmentCorrector range calculation bug, producing
# 0 offenses. We skip to match observable behavior.
module Redcar
  class EditView
    def check; end
  end
end
