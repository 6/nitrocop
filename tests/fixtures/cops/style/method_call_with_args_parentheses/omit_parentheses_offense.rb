# omit_parentheses variant

# Hash-value-omission calls are still offenses as the last expression.
foo(value:)
foo(arg)
   ^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.

child_errors.map do |(error, i)|
  name = attach_child_name(:"#{filter.name}[#{i}]", error)
                          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
  Filter::Error.new(error.filter, error.type, name: name)
                   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
end.freeze

let!(:project) { FactoryBot.create(:project, inactive:, main_language: language) }
                                  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
