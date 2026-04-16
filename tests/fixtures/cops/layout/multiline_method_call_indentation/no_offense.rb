foo.bar.baz

foo
  .bar

foo
  .bar
  .baz

obj.method1
   .method2
   .method3

# In method body
def foo
  query
    .select('foo')
    .limit(1)
end

# Block-based expect chain
expect(response)
  .to have_http_status(200)
  .and have_http_link_header('http://example.com')

# Block chain continuation: aligned with the block-bearing call's dot
frequencies.map.with_index { |f, i| [f / total, hex[i]] }
               .sort_by { |r| -r[0] }
               .reject { |r| r[1].size == 8 }

# Hash pair value chain: correctly aligned
foo(bar: baz
         .qux
         .quux)

# Chain inside parenthesized args without hash value (RuboCop skips these)
foo(baz
      .qux
        .quux)

# Hash pair value: chain after single-line block aligns with block-call dot
foo(bar: items.reject { |e| e.nil? }
              .sort_by(&:name)
              .map(&:id))

# Hash pair value: continuation dot aligned with first inline dot (3+ chain)
method(key: template.submissions.where(x: 1)
                    .or(template.submissions.where(y: 2)))

# Sub-chain starting on a continuation dot line (indented style)
# The `.to` line is a continuation dot; base should be the non-dot ancestor
expect(subject)
  .to receive(:method)
  .and_return(value)

result =
  Foo
  .where(active: true)
  .order(:name)

# Trailing dot style: properly indented
a.
  b

# Trailing dot: no extra indentation of third line
a.
  b.
  c

# Aligned methods in assignment
formatted_int = int_part
                .to_s
                .reverse

# Aligned method in return
def a
  return b.
         c
end

# Aligned method in assignment + block + assignment
a = b do
  c.d = e.
        f
end

# Correctly aligned trailing dot in assignment
a = b.c.
    d

# Inside grouped expression (rubocop skips)
(a.
 b)

# Method chain with hash literal receiver
{ a: 1, b: 2 }.keys
              .first

# Aligned methods in if condition
if a.
   b
  something
end

# Accept indented method when nothing to align with
expect { custom_formatter_class('NonExistentClass') }
  .to raise_error(NameError)

# Indented methods in LHS of []= assignment
a
  .b[c] = 0

# RHS of []= assignment: trailing-dot continuation aligns with chain root,
# not the block-bearing receiver dot
foo[bar] =
  baz.map { |x| [x, qux[x]&.first] }.
  sort_by { |x, _| x.id }.to_h

# Method call chain starting with implicit receiver
def slugs(type, path_prefix)
  expanded_links_item(type)
    .reject { |item| item["base_path"].nil? }
    .map { |item| item["base_path"] }
end

# Aligned methods in operator assignment
a +=
  b
  .c

# 3 aligned methods
a_class.new(severity, location, 'message', 'CopName')
       .severity
       .level

# Aligned method even when an aref is in the chain
foo = '123'.a
           .b[1]
           .c

# Method chain with multiline parenthesized receiver
(a +
 b)
  .foo
  .bar

# Aligned methods in constant assignment
A = b
    .c

# Methods being aligned with method that is an argument
authorize scope.includes(:user)
               .where(name: 'Bob')
               .order(:name)

# Continuation dot aligned with ancestor dot on line directly above
# (RuboCop's get_dot_right_above walks ancestors, not just receiver chain)
expect { subject.run! }
  .to emit_notification("run").with_payload(x)
  .and emit_notification("count").with_payload(y).with_value(0)
  .and emit_notification("future").with_payload(z).with_value(0)
