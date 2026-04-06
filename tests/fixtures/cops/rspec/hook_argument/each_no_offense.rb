# nitrocop-config: EnforcedStyle: each
# Other scope symbols (not :each/:example) are not flagged by RuboCop
before(:create) { create_user }
after(:build) { build_object }
before(:context) { setup_context }
after(:suite) { teardown_suite }

# Multi-arg hooks — RuboCop's NodePattern only matches single-arg scopes
before(:each, :special_tag) do
  setup
end

around(:each, :allow_forgery_protection) do |example|
  example.run
end

# Explicit block-pass is not a block hook
state.before(:each, &handler)