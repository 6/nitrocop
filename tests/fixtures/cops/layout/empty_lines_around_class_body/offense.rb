# coding: US-ASCII
class Foo

^ Layout/EmptyLinesAroundClassBody: Extra empty line detected at class body beginning.
  def bar; end

^ Layout/EmptyLinesAroundClassBody: Extra empty line detected at class body end.
end
class Bar

^ Layout/EmptyLinesAroundClassBody: Extra empty line detected at class body beginning.
  X = 1
end
class Baz
  Y = 2

^ Layout/EmptyLinesAroundClassBody: Extra empty line detected at class body end.
end
class << self

^ Layout/EmptyLinesAroundClassBody: Extra empty line detected at class body beginning.
  def foo
  end

^ Layout/EmptyLinesAroundClassBody: Extra empty line detected at class body end.
end
class Qux
  class << self

^ Layout/EmptyLinesAroundClassBody: Extra empty line detected at class body beginning.
    def bar
    end
  end
end
class MultilineParent <
  BaseClass

^ Layout/EmptyLinesAroundClassBody: Extra empty line detected at class body beginning.
  def method
  end
end
class MultilineParentEnd <
  BaseClass
  def method
  end

^ Layout/EmptyLinesAroundClassBody: Extra empty line detected at class body end.
end
class MultilineBoth <
  BaseClass

^ Layout/EmptyLinesAroundClassBody: Extra empty line detected at class body beginning.
  def method
  end

^ Layout/EmptyLinesAroundClassBody: Extra empty line detected at class body end.
end
class SafeHighBitRange

^ Layout/EmptyLinesAroundClassBody: Extra empty line detected at class body beginning.
  REGEX = /[\x80-\xfd]/n
end
class SafeHighBitRangeEnd
  REGEX = /[\x80-\xfd]/n

^ Layout/EmptyLinesAroundClassBody: Extra empty line detected at class body end.
end
