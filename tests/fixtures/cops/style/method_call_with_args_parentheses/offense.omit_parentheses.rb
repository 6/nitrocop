# nitrocop-config: EnforcedStyle: omit_parentheses

# chained outer calls do not make nested block bodies argument context
expect do
  Class.new(TestInteraction) do
           ^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
    not_a_valid_filter_type :thing
  end
end.to raise_error NoMethodError

# multi-statement block bodies do not inherit a direct block parent
items.map do |item|
  children.push(result)
               ^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
  result.value
end

# case/in branch bodies are still ordinary expressions
case params&.to_h
in { only: }
  Attachment.file_types(from_extensions: only)
                       ^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
end

# Assignment RHS inside multi-statement block body inside a single-statement
# conditional branch must still flag — the block body isolates the assignment
# from the conditional context.
def array_input_errors
  if @index_errors
    child_errors.map do |(error, i)|
      name = attach_child_name(:"#{@filter.name}[#{i}]", error)
                              ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
      Filter::Error.new(error.filter, error.type, name: name)
                       ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
    end.freeze
  end
end

# Multi-statement block body where the outer call is the value of an
# assignment must still flag a non-assignment call.
def array_input_value
  unless filters.empty?
    value = value.map do |item|
      result = filters[:'0'].process(item, context)
                                    ^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
      children.push(result)
                   ^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
      result.value
    end
  end
end

# Nested block: outer is chained into another call (allowed argument
# context), but the inner block's body must reset that context.
def render_tags
  sorted_tags.map do |tag|
    content_tag("li") do
               ^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
      link_to("text", "url")
             ^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
    end
  end.join.html_safe
end
