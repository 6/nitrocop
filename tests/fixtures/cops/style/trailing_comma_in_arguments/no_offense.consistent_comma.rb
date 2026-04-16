# nitrocop-config: EnforcedStyle: consistent_comma

# Multiline call with trailing comma on each own line — OK
foo(
  1,
  2,
  3,
)

# Single-line call without trailing comma — OK
bar("hello", "world")
baz()

# Multiline call where all keyword args are on one line with closing paren —
# treated as a single element, allowed_multiline_argument exemption applies
DateTimeIndex.date_range(
  :start => '2017-4-14', :freq => 'MB', :periods => 5)

# Same pattern with mixed hash rocket and symbol keys
foo(
  a: 1, b: 2, c: 3)

# Index compound assignment — single-line args on same line as method
hash[key] ||= "value"

# Multiline index read with trailing comma — OK
hash[
  key,
]
