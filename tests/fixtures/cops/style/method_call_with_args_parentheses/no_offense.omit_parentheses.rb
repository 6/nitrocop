# nitrocop-config: EnforcedStyle: omit_parentheses

# ambiguity in the receiver chain keeps parentheses
def ignored_organisations_string
  (ignored_organisations || []).join(", ")
end

def convert_grouped_input(value)
  %w[1 2 3].map { |key| value[key] }.join("-")
end

# inner calls inside blocks keep parentheses when the block is directly wrapped
# by another call
self.result = run_callbacks(:execute) do
  execute
end

# inner calls used as super arguments keep parentheses
super(errors.local_attribute(attribute))

# unary descendant arguments keep the outer call parentheses
def input_field
  some_helper(form_field_name, value, id: form_field_id, readonly: !editable?)
end

# lambda descendants make the outer call ambiguous enough to keep parens
options = {
  tag: :div,
  tags_formatter: ->(tags) { tags.join(" ") }
}.merge(options)

# Lambda body whose parent expression is a setter/assignment-like call: the
# lambda block's parent is a call, so RuboCop's call_in_argument_with_block?
# allows parentheses for the inner call.
options[:converter] = ->(x) { ObjectThing.converter(x) }

# Negative-numeric range descendants are ambiguous in the same way as bare
# negative numbers, so the outer call's parens stay.
def slice_domain
  s[1..-1].join(".")
end

# Negative numeric inside an exclusive range descendant
def trim_extension
  Concurrent::Array.new(nested_property_names[0...-1])
end

# Ternary inside a string interpolation descendant of an argument is still
# ambiguous content; the outer call keeps parentheses.
def ordered_languages
  Language.published.order("name #{options[:reverse] ? "DESC" : "ASC"}")
end
