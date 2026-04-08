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

# Outer heredoc closing delimiter with spaces should be flagged
# even when the heredoc contains nested inner heredocs
def wrapper
	<<~EOH
		text
#{if true
    <<-EOI
		content
    EOI
  end}
        EOH
^ Layout/IndentationStyle: Space detected in indentation.
end
