foo.
  bar.
  baz

something.
  chain.
  another

# Single-line is always fine
foo.bar.baz

# .() call in trailing position is fine
foo.
  (arg)

# Scope resolution is not a method call
Foo::Bar
