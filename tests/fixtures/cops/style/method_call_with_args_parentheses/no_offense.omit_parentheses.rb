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
