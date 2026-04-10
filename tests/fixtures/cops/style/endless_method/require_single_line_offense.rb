def my_method
^^^^^^^^^^^^^ Style/EndlessMethod: Use endless method definitions for single line methods.
  x
end

# Operator methods ending with = are NOT assignment methods
def ==(other)
^^^^^^^^^^^^^ Style/EndlessMethod: Use endless method definitions for single line methods.
  x == other.x
end

# Interpolated heredoc as argument: RuboCop's use_heredoc? only checks
# :str descendants, not :dstr, so interpolated heredocs inside other
# expressions are not detected.
def method_with_interp_heredoc_arg
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/EndlessMethod: Use endless method definitions for single line methods.
  super(<<~MSG)
    hello #{name}
  MSG
end

# Block with braces on same line but receiver spanning multiple lines:
# RuboCop's BlockNode#single_line? checks only the block delimiters,
# so { } on the same line counts as single-line even if the receiver
# is multiline.
def multiline_receiver_single_line_block
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/EndlessMethod: Use endless method definitions for single line methods.
  [a,
    b].find { |k| k }
end

# Non-ASCII body: RuboCop's `.length` counts characters, not bytes.
# Cyrillic chars are 2 bytes each in UTF-8, so byte-based length would
# overcount. `def test_street_address_ua_fn = assert_match(...)` is
# 114 chars < 120 but 131 bytes > 120. Must use char count like RuboCop.
def test_street_address_ua_fn
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/EndlessMethod: Use endless method definitions for single line methods.
  assert_match(/\Aвул\.\s[а-яА-ЯіїєґІЇЄҐ'\-\s]+,\s\d{1,3}\z/, @tester.street_address)
end

# Non-parenthesized params: RuboCop's `arguments.source` does NOT include
# a leading space, so `def foobar = body` has no extra space between name
# and args in the length computation. At boundary lengths this 1-char
# difference matters.
def apply_vector_operator operator, vector, other
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/EndlessMethod: Use endless method definitions for single line methods.
  BoolArray.new(vector.zip(other).map { |d, o| !!d.send(operator, o) })
end
