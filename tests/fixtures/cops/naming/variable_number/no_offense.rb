foo1 = 1
bar2 = 2
baz12 = 3
capture3 = 4
iso8601 = 5
x86_64 = 6
def method1; end
:sym1
def func(arg1); end
_1 = 'implicit param'
disable_2fa = true
:disable_2fa
def method_2fa; end
# Symbols/methods ending with ? or ! after digits are valid because the
# suffix is a non-digit character that satisfies the \D regex alternative
:ipv4?
def ipv4?; end
# Keyword parameters are not checked by RuboCop (no on_kwarg/on_kwoptarg)
def foo(bar_1:); end
def foo(baz_2: nil); end
# Integer symbols are valid (all-digit names pass the format regex)
:"42"
%i[1 2 3]
# Standalone empty symbols — Parser gem always creates :dsym for these,
# even with TargetRubyVersion 4.0, so RuboCop's on_sym never fires.
:''
:""
# Special global $$ (PID) — bare name is empty after sigil stripping
$$ = 1
# Pattern matching variable bindings (match_var in Parser, LocalVariableTargetNode
# in Prism) are NOT checked by RuboCop (on_lvasgn doesn't fire for match_var nodes)
case [1, 2]
in [a_1, b_2]
end
value => result_1
obj => { key: val_1 }
# Pattern matching hash keys — in Parser gem, `k_1:` in `value => k_1:`
# creates match_var nodes (not sym), so RuboCop's on_sym never fires.
case value
in { k_1:, k_2: }
  k_1
end
weight = 1.0
weight => k_1:, k_2:, k_l:
# Rescue exception variables with normalcase numbers are fine
begin
rescue => error2
end
# Empty %s() symbols — Parser gem creates :dsym for these, so
# RuboCop's on_sym never fires. Non-empty %s(foo) IS checked.
x = %s()
[%s(), %s()]
