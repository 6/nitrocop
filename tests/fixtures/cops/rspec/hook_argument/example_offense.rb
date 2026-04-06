# nitrocop-config: EnforcedStyle: example
# :each is wrong when EnforcedStyle is :example
before(:each) { true }
^^^^^^^^^^^^^ RSpec/HookArgument: Use `before(:example)` instead of `before(:each)`.
after(:each) { true }
^^^^^^^^^^^^ RSpec/HookArgument: Use `after(:example)` instead of `after(:each)`.
around(:each) { true }
^^^^^^^^^^^^ RSpec/HookArgument: Use `around(:example)` instead of `around(:each)`.

# :example is correct — not flagged
before(:example) { true }
after(:example) { true }
around(:example) { true }

# :suite/:context/:all are fine
before(:suite) { true }
after(:context) { true }

# Implicit usage (no args) should be flagged
before { true }
^^^^^^^^ RSpec/HookArgument: Use `before(:example)` instead of `before`.
after { true }
^^^^^ RSpec/HookArgument: Use `after(:example)` instead of `after`.
around { true }
^^^^^^ RSpec/HookArgument: Use `around(:example)` instead of `around`.