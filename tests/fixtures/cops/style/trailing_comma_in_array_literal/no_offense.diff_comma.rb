# nitrocop-config: EnforcedStyle: diff_comma

# Corpus FP: `diff_comma` accepts a trailing comma when the last item is
# followed by an immediate newline.
multiple_functions = [
  :scanVariable   => [],
  :scanQuotelike  => [],
  :scanCodeblock  => [],
]

array = [
  1, 2,
  3,
]
