# nitrocop-config: EnforcedStyle: require_no_parentheses

def foo(x, y)
       ^^^^^^ Style/MethodDefParentheses: Use `def` without parentheses.
  x + y
end

def bar(x)
       ^^^ Style/MethodDefParentheses: Use `def` without parentheses.
  x
end
