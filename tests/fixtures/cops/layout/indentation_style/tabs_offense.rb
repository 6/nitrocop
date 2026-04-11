# nitrocop-config: EnforcedStyle: tabs

# Space-indented code should be flagged
def foo
  bar
^ Layout/IndentationStyle: Space detected in indentation.
end

# Space-indented xstring content should be flagged (RuboCop checks xstr)
x = `
  command
^ Layout/IndentationStyle: Space detected in indentation.
`

# Space-indented heredoc closing delimiter should be flagged
execute <<-SQL
	SELECT * FROM users
  SQL
^ Layout/IndentationStyle: Space detected in indentation.

# Outer heredoc closing delimiters should still be flagged when nested ones are ignored
x = <<~OUTER
  #{foo(<<~INNER)}
    body
  INNER
  OUTER
^ Layout/IndentationStyle: Space detected in indentation.

# Space-indented dynamic symbol content should be flagged
x = :"foo
  .#{bar}"
^ Layout/IndentationStyle: Space detected in indentation.
