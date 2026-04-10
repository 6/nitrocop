# omit_parentheses variant

# Assignment RHS calls in conditional branches keep parentheses.
if cond
  @calendar = Calendar.new(giftable_dates, user_gifts)
else
  flash[:notice] = t('unsubscribe.invalid_token')
end

# Hash-value-omission calls keep parentheses in conditionals.
do_something(value:) if condition
