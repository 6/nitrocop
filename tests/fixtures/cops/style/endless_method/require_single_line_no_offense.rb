# coding: US-ASCII

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

# Parser-gem rejects this file because of the US-ASCII magic comment combined
# with non-ASCII byte escapes, so Style/EndlessMethod never runs in RuboCop.
class ParseErrorExample
  def setup
    @logger = Logger.new(nil)
  end

  class Log
    def initialize(line)
      /\A(\w+), \[([^#]*) #(\d+)\]\s+(\w+) -- (\w*): ([\x0-\xff]*)/ =~ line
    end
  end
end
