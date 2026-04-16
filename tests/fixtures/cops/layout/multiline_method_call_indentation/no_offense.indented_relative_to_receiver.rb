# nitrocop-config: EnforcedStyle: indented_relative_to_receiver
# Basic chain: receiver at col 0, dot at col 2
a
  .b
  .c

# Chain in method body
def foo
  query
    .select('foo')
    .limit(1)
end

# Assignment: receiver-relative indent
myvariable = Thing
               .a
               .b
               .c

# Multi-line assignment (= on previous line)
result =
  int_part
    .abs
    .to_s

# obj.attr assignment with newline after =
obj.attr =
  int_part
    .abs
    .to_s

# Operation RHS: receiver-relative indent
1 + a
      .b
      .c

# Hash access receiver
hash[:key]
  .do_something

# Trailing dot: proper indent
a.
  b

# Trailing dot: 3rd line same indent as 2nd
a.
  b.
  c

# RSpec block chain continuation
expect { Foo.new }.to change { Bar.count }
                        .from(1).to(2)

# Splat with method chain (width adjusted: 2-1=1)
[
  *foo
    .bar(
      arg)
]

# Double splat with method chain (width adjusted: 2-2=0)
[
  **foo
    .bar(
      arg)
]

# Splat with block chain
[
  *foo
    .bar { |arg| baz(arg) }
]

# Single-line block chain
(0..foo).bar { baz }
          .qux { quux }

(0..foo).bar { baz }
          .qux

(0..foo).bar
          .qux { quux }

# Hash literal receiver: indent relative to first dot+selector
{ a: 1, b: 2 }.keys
                .first

# Parenthesized expression receiver
def run
  (date_columns + candidate_columns).uniq
                                      .select { |cn| castable?(cn) }
                                      .each { |cn| cast(cn) }
end

# Multiline parens: receiver-relative to first dot+selector
(a +
 b).foo
     .bar

# Multi-dot chain in hash pair value passed to method
method(key: value.foo.bar
              .baz)

# Indented methods in LHS of []= assignment
a
  .b[c] = 0

# Indented methods inside and outside a block
a = b.map do |c|
  c
    .b
    .d do
      x
        .y
    end
end

# Indentation relative to first receiver
node
  .children.map { |n| string_source(n) }.compact
  .any? { |s| preferred.any? { |d| s.include?(d) } }

# Indented methods in ordinary statement (trailing dot)
a.
  b

# No extra indentation of third line (trailing dot)
a.
  b.
  c

# Indented methods in for body
for x in a
  something.
    something_else
end

# Alignment inside a grouped expression
(a.
 b)

# An expression where the first method spans multiple lines
subject.each do |item|
  result = resolve(locale) and return result
end.a

# Any indentation of parameters to #[]
payment = Models::IncomingPayments[
        id:      input['incoming-payment-id'],
           user_id: @user[:id]]

# Correctly indented method after hash access
hash[:key]
  .do_something
