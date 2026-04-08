# nitrocop-config: EnforcedStyle: assign_inside_condition

# No else branch — not flagged
x = if foo
      1
    end

# case without else — not flagged
x = case foo
    when "a"
      1
    when "b"
      2
    end

# Already assigns inside condition (the "good" pattern)
if foo
  x = 1
else
  x = 2
end

# Multi-line branches with SingleLineConditionsOnly=true (default) — not flagged
x = if foo
      something
      1
    else
      2
    end

x = if foo
      1
    else
      something_else
      2
    end

# Method call that looks like ternary but is not assignment
bar << foo? ? 1 : 2

# Simple non-conditional assignment
x = 1
@y = 2
$z = "hello"
