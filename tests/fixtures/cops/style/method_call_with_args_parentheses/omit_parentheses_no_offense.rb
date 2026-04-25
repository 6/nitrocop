# omit_parentheses variant

# Assignment RHS calls in conditional branches keep parentheses.
if cond
  @calendar = Calendar.new(giftable_dates, user_gifts)
else
  flash[:notice] = t('unsubscribe.invalid_token')
end

# Hash-value-omission calls keep parentheses in conditionals.
do_something(value:) if condition

# yield with parens inside an assignment whose conditional ancestor exempts
# the call via assignment_in_condition?. RuboCop aliases on_yield to on_send,
# so yield must consult the same legitimate_call_with_parentheses guards.
def deep_each(hash)
  hash.each_pair do |key, value|
    key, value = yield(key, value) unless key == :WechatSession
  end
end

# Single-statement if-branch: assignment_in_condition? exempts the inner yield.
def with_block(value)
  if block_given?
    inspected = yield(value)
  else
    inspected = value.inspect
  end
end

# Single-line ||= parent: RuboCop's require_parentheses_for_hash_value_omission?
# returns true via `node.parent&.single_line?`, so parens stay.
def current_api_user
  @current_api_user ||= User.find_by(api_key:)
end
