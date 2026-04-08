# nitrocop-config: EnforcedStyle: always
# FN fix: named param references inside nested blocks must be found.
# RuboCop's find_block_variables uses each_descendant(:lvar) which
# traverses into nested blocks.

# Named param used directly (no nesting)
block { |x| do_something(x) }
                         ^ Style/ItBlockParameter: Use `it` block parameter.

# Named param used inside nested block
items.each do |item|
  context "something" do
    puts item
         ^^^^ Style/ItBlockParameter: Use `it` block parameter.
  end
end

# Named param captured inside nested lambda
foo.each do |x|
  -> { x.to_s }
       ^ Style/ItBlockParameter: Use `it` block parameter.
end

# Named param in string interpolation inside nested block
%w(a b).each do |frequency|
  context "subscribed to #{frequency} emails" do
                           ^^^^^^^^^ Style/ItBlockParameter: Use `it` block parameter.
    before do
      subject.email_frequency = frequency
                                ^^^^^^^^^ Style/ItBlockParameter: Use `it` block parameter.
    end
  end
end

# Named param used both at top level and inside nested block
items.each do |item|
  item.validate
  ^^^^ Style/ItBlockParameter: Use `it` block parameter.
  nested { item.save }
           ^^^^ Style/ItBlockParameter: Use `it` block parameter.
end

# Lambda with parenthesized named param inside nested block
-> (x) { bar { x.to_s } }
               ^ Style/ItBlockParameter: Use `it` block parameter.
