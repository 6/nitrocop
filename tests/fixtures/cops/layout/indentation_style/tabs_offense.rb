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

# The outer closing delimiter should still be flagged after nested heredocs
# inside interpolation, even though the inner delimiter is suppressed.
msg = <<~OUTER
  #{
    helper(<<~INNER)
      payload
    INNER
  }
  OUTER
^ Layout/IndentationStyle: Space detected in indentation.
