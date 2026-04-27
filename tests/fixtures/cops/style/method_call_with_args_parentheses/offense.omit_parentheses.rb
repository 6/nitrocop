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

# Hash-value-omission calls inside a method body still flag even when the
# enclosing class body has later sibling methods.
class Demo
  def one(value)
    build(value:)
         ^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
  end

  def two
    nil
  end
end

# Lambda call-argument bodies only allow direct-body calls, not calls nested
# under case/in branch wrappers.
add_alchemy_filter :by_file_type, type: :select, options: ->(_query, params) do
  case params&.to_h
  in { only: }
    Attachment.file_types(from_extensions: only)
                         ^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
  end
end

# Hash-value-omission inside a block whose surrounding expression is a
# non-call (here `||=` op-assignment): RuboCop's call_in_argument_with_block?
# rejects the outer or-asgn, and require_parentheses_for_hash_value_omission?
# falls through because the call IS the last expression in the block body.
# Earlier nitrocop kept these parens because the outer non-last `if` leaked a
# non-zero `non_last_expression_depth` into the block body.
def search_filter_params
  @cache ||= begin
    a = 1

    if cond
      bar[:k] ||= [1, 2, 3].map do |extension|
        Marcel::MimeType.for(extension:)
                            ^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
      end
    end

    z
  end
end

# Multi-statement when body wraps as a synthetic `:begin` in Parser AST. The
# inner lvasgn's grandparent in RuboCop is the synthetic begin, NOT the
# outer `if`'s ConditionalBody, so `assignment_in_condition?` returns false
# and the call still flags. Earlier nitrocop let `ConditionalBody` leak
# through as the lvasgn's grandparent.
def picture_factory(picture, acc)
  if acc.image_file
    case Alchemy.storage_adapter.name
    when :active_storage
      filename = acc.image_file.original_filename
      content_type = Marcel::MimeType.for(extension: File.extname(filename))
                                         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
      picture.image_file = filename
    end
  end
end

# `and` / `or` keyword operators are NOT logical_operator? in rubocop-ast
# (only `&&` and `||` are). Calls under an `and`/`or` parent must therefore
# still flag because `call_in_logical_operators?` returns false. nitrocop
# previously treated all AndNode/OrNode parents as logical, exempting these.
def textual?(object)
  object.is_a?(Symbol) or object.is_a?(String)
              ^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
                                      ^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
end

def can_interpret?(parent, keyword, *args, &block)
  (
    keyword == 'bind' and
      (
        (
          (args.size == 1)
        ) ||
        (
          (args.size == 2) and
            textual?(args[1])
                    ^^^^^^^^^ Style/MethodCallWithArgsParentheses: Omit parentheses for method calls with arguments.
        )
      )
  )
end

# `&&` / `||` STILL exempt their operands via `call_in_logical_operators?`.
# Sanity check that semantic-vs-logical distinction does not over-fire.
def textual_and?(object)
  object.is_a?(Symbol) && object.is_a?(String)
end
