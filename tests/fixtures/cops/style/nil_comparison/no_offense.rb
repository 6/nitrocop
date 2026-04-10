x.nil?

y.nil?

x == y

x == 0

x.is_a?(NilClass)

# Comparison with non-nil values
x == false
x === String
x == ""

# Method calls that are not comparisons
x.nil_class
x.nill?

# comparison style: RuboCop ignores safe-navigation nil checks
# nitrocop-config: EnforcedStyle: comparison
headers[:twr] = region unless region&.nil?
