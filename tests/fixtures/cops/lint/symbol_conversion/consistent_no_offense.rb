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

# FP fix: symbols inside %i arrays should NOT trigger is_in_undef
# even when the word "undef" appears as a sibling in the array.
# rubocop/rubocop redundant_self.rb and send_with_literal_method_name.rb patterns.
%i[alias and begin break case class def defined? do
   else elsif end ensure false for if in module
   next nil not or redo rescue retry return self
   super then true undef unless until when while
   yield]

# Bare symbols already matching correction are not flagged
:foo
:~
:+
