# omit_parentheses variant

# Hash-value-omission calls are still offenses as the last expression.
foo(value:)
foo(arg)
   ^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.

let!(:project) { FactoryBot.create(:project, inactive:, main_language: language) }
                                  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
