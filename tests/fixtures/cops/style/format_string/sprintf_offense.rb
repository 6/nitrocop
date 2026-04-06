# nitrocop-config: EnforcedStyle: sprintf
format(something, a, b)
^^^^^^ Style/FormatString: Favor `sprintf` over `format`.
"%d" % 10
     ^ Style/FormatString: Favor `sprintf` over `String#%`.
x % [10, 11]
  ^ Style/FormatString: Favor `sprintf` over `String#%`.
x % { a: 10, b: 11 }
  ^ Style/FormatString: Favor `sprintf` over `String#%`.
"%f" % a
     ^ Style/FormatString: Favor `sprintf` over `String#%`.
"#{x * 5} %d #{@test}" % 10
                       ^ Style/FormatString: Favor `sprintf` over `String#%`.
format("%X", 123)
^^^^^^ Style/FormatString: Favor `sprintf` over `format`.
