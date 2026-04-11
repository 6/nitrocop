# omit_parentheses variant

# Assignment RHS calls in conditional branches keep parentheses.
if cond
  @calendar = Calendar.new(giftable_dates, user_gifts)
else
  flash[:notice] = t('unsubscribe.invalid_token')
end

# Hash-value-omission calls keep parentheses in conditionals.
do_something(value:) if condition

# Receiver-side ambiguity also allows parentheses.
(ignored_organisations || []).join(", ")

[*only].include?(k)

# Calls used as super arguments keep their inner parentheses.
def read_attribute_for_validation(attribute)
  super(errors.local_attribute(attribute))
end
