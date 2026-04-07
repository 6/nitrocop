# nitrocop-config: EnforcedStyle: require_no_parentheses_except_multiline

# Single-line args without parens: OK
def foo x, y
  x + y
end

# No args: OK
def bar
  42
end

# Multiline args with parens: OK (parens required for multiline)
def qux(x,
        y)
  x + y
end

# Endless method with parens: OK (forced parens)
def baz(x) = x + 1

# Forwarding parameter: OK (forced parens)
def fwd(...)
  other(...)
end

# Anonymous rest: OK (forced parens)
def anon_rest(*)
end

# Anonymous kwrest: OK (forced parens)
def anon_kwrest(**)
end

# Anonymous block: OK (forced parens)
def anon_block(&)
  yield
end
