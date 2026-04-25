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

# Hash-value-omission inside a multi-line `do...end` block fires: parent
# block isn't conditional or single-line, and the call is the last statement.
# The outer `||= begin ... end` wrapper must not exempt inner calls.
def search_filter_params
  @_search_filter_params ||= begin
    if params[:only].present?
      params[:q][:by_file_type] ||= Array(params[:only]).map do |extension|
        Marcel::MimeType.for(extension:)
                            ^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
      end
    end
  end
end

# `||= begin ... end.to_sym` — the trailing `join("_")` is the begin's last
# statement. Its direct parent is the begin (kwbegin), which RuboCop does not
# exempt, so the parens should be flagged. The intermediate `delete(...)` is
# not a chain receiver — its direct parent is the modifier `if`.
def namespaced_resources_name
  @_namespaced_resources_name ||= begin
    resource_name_array = resource_array.dup
    resource_name_array.delete(engine_name) if in_engine?
                              ^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
    resource_name_array.join("_")
                            ^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
  end.to_sym
end

# `case ... else <call> end.transform_values!` — RuboCop exempts `when` bodies
# via `parent.when_type?`, but the `else` branch's parent is the case node, so
# parens there must still be flagged.
def resize_options(size_string, width, height, options)
  case size_string
  when RESIZE_TO_FIT
    resize_to_fit_options(width, height)
  else
    resize_to_limit_options(width, height)
                           ^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
  end.transform_values! do |value|
    value.push(sharpen_option(options))
              ^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
  end
end
