# nitrocop-config: EnforcedStyle: require_no_parentheses

def foo(x, y)
       ^^^^^^ Style/MethodDefParentheses: Use `def` without parentheses.
  x + y
end

def bar(x)
       ^^^ Style/MethodDefParentheses: Use `def` without parentheses.
  x
end

def empty()
         ^^ Style/MethodDefParentheses: Use `def` without parentheses.
  42
end

# **nil (no-keywords parameter) does NOT force parentheses
def no_kwargs(**nil)
             ^^^^^^^ Style/MethodDefParentheses: Use `def` without parentheses.
end

# **nil with named block arg
def no_kwargs_block(**nil, &b)
                   ^^^^^^^^^^ Style/MethodDefParentheses: Use `def` without parentheses.
  b
end

# positional arg with **nil
def pos_no_kwargs(a, **nil)
                 ^^^^^^^^^^ Style/MethodDefParentheses: Use `def` without parentheses.
  a
end
