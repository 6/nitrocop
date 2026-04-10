# nitrocop-config: EnforcedStyle: require_parentheses_when_complex

# Non-complex condition with parens — should flag
(foo) ? bar : baz
^ Style/TernaryParentheses: Only use parentheses for ternary expressions with complex conditions.

# Method call with argument — non-complex but parenthesized
(val.is_a? OptionGroup) ? val.to_rpc_data : val
^ Style/TernaryParentheses: Only use parentheses for ternary expressions with complex conditions.

# Complex condition without parens — should flag
x && y ? 1 : 0
^ Style/TernaryParentheses: Use parentheses for ternary expressions with complex conditions.

# Calls with real blocks are complex without parens
[1, 2, 3].any? { |value| value.odd? } ? :odd : :even
^ Style/TernaryParentheses: Use parentheses for ternary expressions with complex conditions.
