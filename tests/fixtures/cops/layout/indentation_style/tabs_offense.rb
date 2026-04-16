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

# With nested heredocs inside interpolation, only the outer closing delimiter
# should be flagged when it is space-indented under tabs style.
def foo
	render html->{ <<~HTML
  <ul>
  #{html_map 3.times do |i| <<~HTML
    <li>#{text->{ i }}</li>
  HTML
  end}
  </ul>
  HTML
^ Layout/IndentationStyle: Space detected in indentation.
	}
end
