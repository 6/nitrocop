# coding: US-ASCII
class Foo
  def bar; end
end
class Bar
  X = 1
end
class Baz
  Y = 2
end
class << self
  def foo
  end
end
class Qux
  class << self
    def bar
    end
  end
end
class MultilineParent <
  BaseClass
  def method
  end
end
class MultilineParentEnd <
  BaseClass
  def method
  end
end
class MultilineBoth <
  BaseClass
  def method
  end
end
class SafeHighBitRange
  REGEX = /[\x80-\xfd]/n
end
class SafeHighBitRangeEnd
  REGEX = /[\x80-\xfd]/n
end
