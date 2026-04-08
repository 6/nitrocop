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
