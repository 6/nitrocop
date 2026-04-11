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

# Nested heredoc closing delimiters (inner) should NOT be flagged
# They are inside the outer heredoc body — RuboCop skips them
method_name = class_eval <<~OUTER, __FILE__, __LINE__ + 1
  def run(data)
    #{if true
      <<~INNER
        code_here
      INNER
    end}
  end
OUTER
