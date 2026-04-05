class RescueSpecs::C
  raise "message"
rescue => e
  ScratchPad << e.message
end

class Foo
  raise "bar"
rescue Baz => ex
end
