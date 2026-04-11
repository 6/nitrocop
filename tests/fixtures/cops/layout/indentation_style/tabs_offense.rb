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

# Outer closing delimiter of nested heredoc should be flagged
class Foo
  def bar
^ Layout/IndentationStyle: Space detected in indentation.
    method_name = class_eval <<~OUTER, __FILE__, __LINE__ + 1
^ Layout/IndentationStyle: Space detected in indentation.
      def run(data)
        #{if true
          <<~INNER
            code_here
          INNER
        end}
      end
    OUTER
^ Layout/IndentationStyle: Space detected in indentation.
  end
^ Layout/IndentationStyle: Space detected in indentation.
end
