def foo
  42
end

def bar
  x = 1
  x
end

def baz(x)
  x + 1
end

# return in terminal position of if/else
def with_if(x)
  if x > 0
    x
  else
    -x
  end
end

# return in terminal position of if/elsif/else
def with_elsif(x)
  if x > 0
    1
  elsif x == 0
    0
  else
    -1
  end
end

# return in terminal position of case/when
def with_case(x)
  case x
  when 1
    :one
  when 2
    :two
  else
    :other
  end
end

# return in terminal position of begin/rescue
def with_rescue
  begin
    do_something
  rescue StandardError
    default_value
  end
end

# return in terminal position of unless
def with_unless(x)
  unless x.nil?
    x
  else
    0
  end
end

# return in nested if inside case
def nested_control(x)
  case x
  when :a
    if true
      1
    else
      2
    end
  else
    3
  end
end

# return in begin/rescue/else/ensure - rescue is the body's last statement
def with_rescue_else
  begin
    try_something
  rescue
    fallback
  end
end

# implicit begin (def body with rescue)
def implicit_rescue
  do_work
rescue
  safe_value
end

# return in block body of define_singleton_method
define_singleton_method(:foo) do
  42
end

# return in lambda body
lambda do
  true
end

# return in brace block of define_singleton_method
define_singleton_method(:bar) { true }

# return in define_method block
define_method(:baz) do
  :result
end

# return in stabby lambda
-> { 42 }

# return with rescue modifier in terminal position
def rescue_modifier_return
  bar rescue nil
end

# return in terminal position of case/in (pattern matching)
def with_case_in(x)
  case x
  in :a
    1
  in :b
    2
  else
    3
  end
end
