# nitrocop-config: EnforcedStyle: require_no_parentheses_except_multiline

# Multiline args without parens: offense (needs parentheses)
def foo x,
        ^ Style/MethodDefParentheses: Use `def` with parentheses when there are parameters.
        y
  x + y
end
