# nitrocop-config: EnforcedStyle: grouped

class Foo
  include Bar
end

class Baz
  extend A
end

class Quux
  prepend X
end

class LexerDsl
  prepend :root do
    rule %r/x/, Name::Tag
  end

  prepend :attr do
    rule %r/y/, Name::Attribute
  end
end
