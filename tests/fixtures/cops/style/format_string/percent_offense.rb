# nitrocop-config: EnforcedStyle: percent
format(something, a)
^^^^^^ Style/FormatString: Favor `String#%` over `format`.
format(something, a, b)
^^^^^^ Style/FormatString: Favor `String#%` over `format`.
format(something, a: 10, b: 11)
^^^^^^ Style/FormatString: Favor `String#%` over `format`.
sprintf(something, a)
^^^^^^^^ Style/FormatString: Favor `String#%` over `sprintf`.
format("%d %04x", 123, 123)
^^^^^^ Style/FormatString: Favor `String#%` over `format`.
sprintf(something, a: 10, b: 11)
^^^^^^^^ Style/FormatString: Favor `String#%` over `sprintf`.