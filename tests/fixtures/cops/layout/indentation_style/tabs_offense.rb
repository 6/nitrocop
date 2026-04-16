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

# Outer closer should still be flagged even when a nested closer in interpolation is ignored
doc = <<~OUTER
  #{
    helper(<<~INNER)
      body
    INNER
  }
    OUTER
^ Layout/IndentationStyle: Space detected in indentation.
