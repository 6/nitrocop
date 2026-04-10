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

# Heredoc with inline trailing dot method — single-line call, no offense
foo = <<~SQL.squish
  SELECT * FROM foo
SQL

# Heredoc chain on same line as opening tag
bar = <<~HTML.html_safe.freeze
  <p>Hello</p>
HTML

# Scope resolution is not a method call
Foo::Bar
