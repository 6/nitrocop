foo
  .bar
  ^ Layout/DotPosition: Place the `.` on the previous line, together with the method call receiver.
  .baz
  ^ Layout/DotPosition: Place the `.` on the previous line, together with the method call receiver.

something
  .chain
  ^ Layout/DotPosition: Place the `.` on the previous line, together with the method call receiver.
  .another
  ^ Layout/DotPosition: Place the `.` on the previous line, together with the method call receiver.

# Comment between receiver and leading-style dot
# RuboCop still flags even when there's a comment or gap between receiver and dot
foo(
  bar,
  baz
)
# a comment here
.method_name
^ Layout/DotPosition: Place the `.` on the previous line, together with the method call receiver.

# Constant receiver with leading dot on non-adjacent line
notified = GroupUser
  # some comment
  .where(user_id: ids)
  ^ Layout/DotPosition: Place the `.` on the previous line, together with the method call receiver.

# .() call syntax (no method name) should still be checked
foo
  .(arg)
  ^ Layout/DotPosition: Place the `.` on the previous line, together with the method call receiver.

# Heredoc receiver - dot should be on previous line (with heredoc end), not on squish line
foo = <<-SQL
  SELECT * FROM foo
SQL
  .squish
  ^ Layout/DotPosition: Place the `.` on the previous line, together with the method call receiver.
