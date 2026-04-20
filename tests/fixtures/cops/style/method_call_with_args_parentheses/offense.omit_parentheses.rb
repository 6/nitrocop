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
