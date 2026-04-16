# nitrocop-config: EnforcedStyle: tabs

# Tab-indented code is fine
def foo
	bar
end

# Spaces inside heredoc body should NOT be flagged
msg = <<~HEREDOC
  content with spaces
HEREDOC

# Spaces inside squiggly heredoc with interpolation should NOT be flagged
expect(x).to eq(y), <<~MSG
  #{model} text
  expected: #{val}
MSG

# Spaces inside regular string should NOT be flagged
x = "hello
  world"

# Space-indented inner nested heredoc closing delimiters remain inside the
# outer heredoc body, so RuboCop does not flag them under EnforcedStyle: tabs.
msg = <<~OUTER
  #{
    helper(<<~INNER)
      payload
    INNER
  }
OUTER
