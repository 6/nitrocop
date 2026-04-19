# omit_parentheses variant

# Hash-value-omission calls are still offenses as the last expression.
foo(value:)
foo(arg)
   ^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.

let!(:project) { FactoryBot.create(:project, inactive:, main_language: language) }
                                  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.

# Only the LAST hash pair's value-omission exempts the call. Here the last
# pair is `main_language: language` (regular), so the call still fires even
# as a non-last expression under a let block.
describe "nested" do
  let!(:other) { FactoryBot.create(:project, inactive:, main_language: language) }
                                  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
  let(:a) { 1 }
end
