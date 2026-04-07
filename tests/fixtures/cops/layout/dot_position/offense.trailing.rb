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

# Heredoc receiver - dot should be on previous line (with heredoc end), not on squish line
foo = <<-SQL
  SELECT * FROM foo
SQL
  .squish
  ^ Layout/DotPosition: Place the `.` on the previous line, together with the method call receiver.
