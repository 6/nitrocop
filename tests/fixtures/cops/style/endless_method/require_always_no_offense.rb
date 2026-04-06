def my_method
  begin
    x
  end
end

def my_other_method
  x
  y
end

# Parenthesized body: parser gem treats `(expr)` as begin_type?,
# so RuboCop's can_be_made_endless? returns false.
def paren_body
  (a + b)
end
