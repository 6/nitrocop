# nitrocop-config: EnforcedStyle: omit_parentheses

# chained outer calls do not make nested block bodies argument context
expect do
  Class.new(TestInteraction) do
           ^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
    not_a_valid_filter_type :thing
  end
end.to raise_error NoMethodError
