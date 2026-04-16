# nitrocop-config: EnforcedStyle: omit_parentheses

# ambiguity in the receiver chain keeps parentheses
def ignored_organisations_string
  (ignored_organisations || []).join(", ")
end

def convert_grouped_input(value)
  %w[1 2 3].map { |key| value[key] }.join("-")
end

# calls directly inside a block are allowed when the block expression is
# nested under another call
expect { foo(1) }.to raise_error(RuntimeError)

# setter calls with block-wrapped values keep the inner call's parentheses
self.result = run_callbacks(:execute) do
  execute
end

# calls used as explicit super arguments keep their parentheses
def read_attribute_for_validation(attribute)
  super(errors.local_attribute(attribute))
end
