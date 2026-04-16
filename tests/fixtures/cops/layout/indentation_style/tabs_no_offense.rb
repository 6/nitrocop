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

# Space-indented inner closing delimiter inside an outer heredoc body should
# not be flagged
def describe
	x = <<~OUTER
	  #{
	    helper('text', <<~INNER)
	      "filter": "selectorgadget",
	    INNER
	  }
OUTER
end
