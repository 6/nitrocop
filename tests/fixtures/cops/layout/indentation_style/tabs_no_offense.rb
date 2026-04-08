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

# Inner heredoc closing delimiter with spaces inside an outer heredoc body
# should NOT be flagged (RuboCop considers it string content via the outer
# heredoc's string_literal_ranges)
def powershell_wrapper
	<<~EOH
		text
#{if true
    <<-EOI
		content
    EOI
  end}
	EOH
end
