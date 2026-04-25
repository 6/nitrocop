x = 1 +
        2
        ^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.
z = 5 +
      6
      ^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.
w = a &&
         b
         ^^^^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.

# Chained || with same-indent continuations (most common FN pattern)
def skip?
  a ||
  b ||
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
  c
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
end

# Multiline && in if condition - misaligned
if a &&
  b
  ^ Layout/MultilineOperationIndentation: Align the operands of a condition in an `if` statement spanning multiple lines.
  do_something
end

# FN: Assignment with chained + continuation at wrong indent
result = foo("h3") +
  foo("p1") +
  ^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.
  foo("p2")
  ^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.

# Same-indent chained + in assignment with wrong indent
result2 = "hello".capitalize +
  "world" +
  ^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.
  "foo"
  ^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.

# Same-column chained + in a method body is still an offense
def lyrics
  "hello".capitalize +
  "world" +
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
  "foo" +
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
  "bar"
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
end

# Operator calls used as method arguments must align in `aligned` style.
puts a, 1 +
  2
  ^ Layout/MultilineOperationIndentation: Align the operands of an expression spanning multiple lines.

it "should convert " +
  "a to " +
  ^^^^^^^ Layout/MultilineOperationIndentation: Align the operands of an expression spanning multiple lines.
  "b" do
  ^^^ Layout/MultilineOperationIndentation: Align the operands of an expression spanning multiple lines.
end

# Boolean operation inside a block body with over-indented operands.
values = fields.map { |attrs| attrs.value }
               .reject { |v| v.empty? ||
                             v == "updatedns" ||
                             ^ Layout/MultilineOperationIndentation: Use 2 (not 14) spaces for indenting an expression spanning multiple lines.
                             v == "Submit"
                             ^ Layout/MultilineOperationIndentation: Use 2 (not 14) spaces for indenting an expression spanning multiple lines.
                       }

# Boolean chains passed as keyword arguments in method calls align to argument start.
it "reports errors", if: RUBY_VERSION < "2.6" ||
  PlatformHelpers.truffleruby? || PlatformHelpers.jruby? &&
  ^ Layout/MultilineOperationIndentation: Align the operands of an expression spanning multiple lines.
    Gem::Version.new(RUBY_ENGINE_VERSION) >= "9.3.7.0" do
    ^ Layout/MultilineOperationIndentation: Align the operands of an expression spanning multiple lines.
end

# Operator inside a case expression that is an assignment RHS still uses
# assignment alignment in aligned style.
def get_date_filter(operator)
  filter = case operator
    when OPERATOR_TODAY
      "BETWEEN" +
        "AND"
        ^ Layout/MultilineOperationIndentation: Align the operands of an expression in an assignment spanning multiple lines.
    end
end
