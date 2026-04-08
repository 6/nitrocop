# nitrocop-config: EnforcedStyle: consistent

# Bare hash keys are handled by check_hash_consistent, not flagged individually
{ foo: 1, bar: 2 }

# %i arrays are not flagged
%i(foo bar baz)
%I(foo bar)

# Bare symbols with : prefix are already canonical
:foo
:bar_baz

# Alias arguments are not flagged
alias foo bar
