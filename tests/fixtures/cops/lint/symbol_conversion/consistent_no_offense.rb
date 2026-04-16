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

# FP fix: "undef" as first word on a line inside %i — the newline makes
# is_undef_at_statement_start return true, falsely treating "unless" as
# a undef argument.
%i[
  undef unless until
]

# FP fix: `%i(` arrays should not be mistaken for `undef` statements either.
# did_you_mean reserved-word lists use paren delimiters, not `[]`.
RB_RESERVED_WORDS = %i(
  super
  then
  true
  undef
  unless
  until
  when
  while
  yield
)

RB_RESERVED_WORDS_WITH_PSEUDOVARS = %i(
  retry
  return
  self
  super
  then
  true
  undef
  unless
  until
  when
  while
  yield
  __LINE__
  __FILE__
  __ENCODING__
)

# FP fix: bare symbol values (not hash keys) should not be checked
# in consistent mode. RuboCop's on_sym skips non-hash-key symbols.
test_gvar(gvar: :$-0)
test_gvar(gvar: :$-F)

# Bare symbols already matching correction are not flagged
:foo
:~
:+
