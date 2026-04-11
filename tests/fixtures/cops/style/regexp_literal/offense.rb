%r_ls_
^^^^^^ Style/RegexpLiteral: Use `//` around regular expression.

%r{foo}
^^^^^^^ Style/RegexpLiteral: Use `//` around regular expression.

%r(bar)
^^^^^^^ Style/RegexpLiteral: Use `//` around regular expression.

/foo\/bar/
^^^^^^^^^^ Style/RegexpLiteral: Use `%r` around regular expression.

PARSING_REGEX = %r{ (?:"(?:[^\\"]|\\.)*") | (?:'(?:[^\\']|\\.)*') | \S+ }x # "
                ^ Style/RegexpLiteral: Use `//` around regular expression.

SCANNING_REGULAR_EXPRESSION = %r{ (?:"(?:[^\\"]|\\.)*") | (?:'(?:[^\\']|\\.)*') | (?:\[(?:[^\\\[\]]|\\.)*\]) | \S+ }x # "
                              ^ Style/RegexpLiteral: Use `//` around regular expression.

%r{#{part} (#{separator} #{part})*}x
^ Style/RegexpLiteral: Use `//` around regular expression.

%r{  -U #{expected_runit_owner}:#{expected_runit_group} \\},
^ Style/RegexpLiteral: Use `//` around regular expression.

%r{  -u #{expected_runit_owner}:#{expected_runit_group} \\},
^ Style/RegexpLiteral: Use `//` around regular expression.

when %r{=}
     ^ Style/RegexpLiteral: Use `//` around regular expression.

FULL_ENCODED_VALUE = %r{ # Identical to ENCODED_VALUE but captures the whole rather than components of
                     ^ Style/RegexpLiteral: Use `//` around regular expression.
  foo
}x

abstract_texts = pdf.find_text %r/line of abstract/
                               ^ Style/RegexpLiteral: Use `//` around regular expression.
