# nitrocop-config: EnforcedStyle: require_no_parentheses_except_multiline

# Single-line args with parens: offense (should not use parentheses)
def foo(x, y)
       ^^^^^^ Style/MethodDefParentheses: Use `def` without parentheses.
  x + y
end

# Single arg with parens
def bar(x)
       ^^^ Style/MethodDefParentheses: Use `def` without parentheses.
  x
end

# Class method with parens
def self.baz(x, y)
            ^^^^^^ Style/MethodDefParentheses: Use `def` without parentheses.
  x + y
end

# Default value with parens
def qux(a, b = 1)
       ^^^^^^^^^^ Style/MethodDefParentheses: Use `def` without parentheses.
  a + b
end

# Named block arg with parens
def with_block(x, &block)
              ^^^^^^^^^^^ Style/MethodDefParentheses: Use `def` without parentheses.
  yield
end

# Multiline args without parens: offense (needs parentheses)
def multi x,
          ^^ Style/MethodDefParentheses: Use `def` with parentheses when there are parameters.
          y
  x + y
end
