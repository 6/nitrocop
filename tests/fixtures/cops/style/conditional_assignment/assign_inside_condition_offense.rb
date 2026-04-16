# nitrocop-config: EnforcedStyle: assign_inside_condition

# Local variable = if/else
x = if foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
      1
    else
      2
    end

# Instance variable = if/else
@x = if foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
       1
     else
       2
     end

# Class variable = if/else
@@x = if foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
        1
      else
        2
      end

# Global variable = if/else
$x = if foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
       1
     else
       2
     end

# Constant = if/else
X = if foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
      1
    else
      2
    end

# Constant path = if/else
FOO::BAR = if baz?
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
               1
           else
               2
           end

# case/when/else
x = case foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
    when "a"
      1
    else
      2
    end

# case/when with multi-statement branch
x = case foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
    when "a"
      something
      1
    else
      2
    end

# unless/else
x = unless foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
      1
    else
      2
    end

# Ternary
x = foo? ? 1 : 2
^ Style/ConditionalAssignment: Assign variables inside of conditionals.

# Setter method = if/else
foo.bar = if baz
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
            1
          else
            2
          end

# Index setter = if/else
foo[:a] = if bar?
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
            1
          else
            2
          end

# Operator assignment
x += if foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
       1
     else
       2
     end

# And assignment
x &&= if foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
        1
      else
        2
      end

# Or assignment
x ||= if foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
        1
      else
        2
      end

# Multi-write
a, b = if foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
         [1, 2]
       else
         [3, 4]
       end

# if/elsif/else
x = if foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
      1
    elsif bar
      2
    else
      3
    end

# if/elsif without else (still flagged)
x = if foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
      1
    elsif bar
      2
    end

# case/in pattern matching
x = case foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
    in "a"
      1
    in "b"
      2
    else
      3
    end

# case/in with multi-statement branch
x = case foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
    in "a"
      something
      1
    else
      2
    end

# Shovel operator assignment to conditional
bar << if foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
         1
       else
         2
       end

# Comparison operator with conditional RHS
bar == if foo
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
        1
      else
        2
      end

# Corpus FN: local variable = inline if/else
user_time_zone = if @new_contribution.user.time_zone.nil? then 'UTC' else @new_contribution.user.time_zone end
^ Style/ConditionalAssignment: Assign variables inside of conditionals.

# Corpus FN: variable = if with respond_to?
language = if language.respond_to?(:map)
^ Style/ConditionalAssignment: Assign variables inside of conditionals.
              language.map(&method(:escape_language))
            else
              escape_language(language)
            end
