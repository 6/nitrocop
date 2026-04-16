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

# case/else with direct multiline else body remains allowed
x = case foo
    when "a"
      1
    else
      something_else
      2
    end

# case/in else with direct multiline else body remains allowed
x = case foo
    in "a"
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

# FP fix: ternary with parenthesized branches (begin_type? in RuboCop)
x = cond ? (a) : b
x = cond ? a : (b)
x = cond ? (a) : (b)
success = (foo ? (bar == '0') : (baz == '1'))

# FP fix: if/else with parenthesized branch expression
x = if foo
      (bar ? 'a' : 'b')
    else
      'c'
    end

# FP fix: assignment nested inside another assignment (part_of_ignored_node?)
y = [1].map { |v| x = v > 0 ? 'pos' : 'neg'; x }
trunc = lambda { |s| s = s.length > 10 ? s : s[0..10]; s }

# FP fix: assignment inside ||= begin...end
@cache ||= begin
  path = windows ? c_chef_dir : other_dir
  clean(path)
end
