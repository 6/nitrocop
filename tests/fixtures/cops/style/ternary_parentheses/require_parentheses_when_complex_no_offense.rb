# nitrocop-config: EnforcedStyle: require_parentheses_when_complex

# Simple condition without parens — OK
foo ? bar : baz

# Complex condition with parens — OK
(x && y) ? 1 : 0

# defined? is non-complex — no parens OK
defined?(Foo) ? Foo : nil

# yield is non-complex — no parens OK
yield ? 1 : 0
