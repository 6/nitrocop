# nitrocop-config: EnforcedStyle: each
# :example is wrong when EnforcedStyle is :each
before(:example) { true }
^^^^^^^^^^^^^^^ RSpec/HookArgument: Use `before(:each)` instead of `before(:example)`.
after(:example) { true }
^^^^^^^^^^^^^^ RSpec/HookArgument: Use `after(:each)` instead of `after(:example)`.
around(:example) { true }
^^^^^^^^^^^^^^^^ RSpec/HookArgument: Use `around(:each)` instead of `around(:example)`.

# :each is correct — not flagged
before(:each) { true }
after(:each) { true }
around(:each) { true }

# :suite/:context/:all are fine
before(:suite) { true }
after(:context) { true }

# Implicit usage (no args) should be flagged
before { true }
^^^^^^^^ RSpec/HookArgument: Use `before(:each)` instead of `before`.
after { true }
^^^^^ RSpec/HookArgument: Use `after(:each)` instead of `after`.
around { true }
^^^^^^ RSpec/HookArgument: Use `around(:each)` instead of `around`.