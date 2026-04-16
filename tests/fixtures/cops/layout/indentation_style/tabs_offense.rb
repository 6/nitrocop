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

# Space-indented outer closing delimiter should still be flagged when an inner
# heredoc closing delimiter appears earlier inside the outer heredoc body
def describe
	x = <<~OUTER
	  #{
	    helper('text', <<~INNER)
	      "filter": "selectorgadget",
	    INNER
	  }
  OUTER
^ Layout/IndentationStyle: Space detected in indentation.
end
