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
