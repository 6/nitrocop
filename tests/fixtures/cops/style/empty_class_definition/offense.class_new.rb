@memory_class = class Testing::MyMemory < Puppet::Indirector::Memory
                ^ Style/EmptyClassDefinition: Prefer `Class.new` over class definition for classes with no body.
  self
end

class MyClass
^ Style/EmptyClassDefinition: Prefer `Class.new` over class definition for classes with no body.
  self
end
