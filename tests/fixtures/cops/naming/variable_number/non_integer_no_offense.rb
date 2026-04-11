# nitrocop-config: EnforcedStyle: non_integer

# Names without trailing digits are valid
foo = 1
bar_baz = 2
def some_method; end
:some_sym

# All-digit symbols are valid (bare name matches \A\d+\z)
:"42"

# Names ending with non-digit are valid
disable_2fa = true
:disable_2fa
def method_2fa; end

# Names ending with ? or ! after digits are valid
:ipv4?
def ipv4?; end
