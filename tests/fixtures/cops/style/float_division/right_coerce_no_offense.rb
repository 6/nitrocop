# nitrocop-config: EnforcedStyle: right_coerce
# These should NOT be flagged because the to_f receiver is a regexp match result

# nth_ref on left - $1 is a regexp match
$1.to_f / b

# nth_ref on right
a / $1.to_f

# Regexp.last_match on left
Regexp.last_match(1).to_f / b

# Regexp.last_match on right
a / Regexp.last_match(1).to_f

# Both sides with regexp match (should still not be flagged for right_coerce because left has regexp match)
$1.to_f / $2.to_f
