def my_method
  x
  y
end

def my_other_method
  begin
    x
  end
end

# Parenthesized body: parser gem treats `(expr)` as begin_type?,
# so RuboCop's can_be_made_endless? returns false.
def paren_body
  (a + b)
end

def paren_body_method_call
  (unsubscribed_members + cleaned_members)
end
