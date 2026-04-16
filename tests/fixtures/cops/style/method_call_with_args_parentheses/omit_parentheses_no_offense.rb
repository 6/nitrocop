# omit_parentheses variant

# Assignment RHS calls in conditional branches keep parentheses.
if cond
  @calendar = Calendar.new(giftable_dates, user_gifts)
else
  flash[:notice] = t('unsubscribe.invalid_token')
end

# Hash-value-omission calls keep parentheses in conditionals.
do_something(value:) if condition

# Parens are allowed when the receiver contains ambiguous descendants.
date = %w[1 2 3].map { |key| value[key] }.join('-')

# Parens are allowed when a block expression is itself an outer call argument.
self.result = run_callbacks(:execute) do
  execute
end
