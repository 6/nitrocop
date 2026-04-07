# nitrocop-config: EnforcedStyle: require_no_parentheses_except_multiline

# Single-line args without parens: OK
def foo x, y
  x + y
end

# No args: OK
def bar
  42
end

# Single-line args with parens: OK
def baz(x, y)
  x + y
end

# Multiline args with parens: OK (parens required for multiline)
def qux(x,
        y)
  x + y
end
