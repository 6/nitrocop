expect(foo).to be_empty
expect(foo).to have_something
expect(foo.something?).to eq "something"
expect(foo.something).to be(true)
expect(foo.has_something).to be(true)
expect(foo).not_to be_empty

# Bare predicate calls without a receiver should not be flagged
# (they are locally-defined helper methods, not predicates on an object)
expect(enabled?('Layout/DotPosition')).to be(false)
expect(enabled?('Layout/EndOfLine')).to be(false)
expect(cop_enabled?(cop_class)).to be true
expect(valid?).to be_truthy
expect(something?).to be_falsey

# Safe navigation (&.) should not be flagged — can't rewrite to predicate matcher
expect(element&.visible?).to be_falsey
expect(record&.active?).to be_truthy

# Explicit style: built-in matchers should NOT be flagged
# nitrocop-config: EnforcedStyle: explicit
expect(foo).to be_truthy
expect(foo).to be_falsey
expect(foo).to be_falsy
expect(foo).to have_received(:bar)
expect(foo).to have_attributes(name: 'foo')
expect(foo).to be_between(1, 10)
expect(foo).to be_within(0.1).of(10)
expect(foo).to exist

# Explicit style: be(true)/be(false) with non-predicate should not be flagged
# nitrocop-config: EnforcedStyle: explicit
expect(foo).to be(true)
expect(foo).to be(false)

# Explicit style: include with no arguments should NOT be flagged
# nitrocop-config: EnforcedStyle: explicit
expect(foo).to include
expect(foo).to include, 'fail'

# Explicit style: include with multiple arguments should NOT be flagged
# nitrocop-config: EnforcedStyle: explicit
expect(foo).to include(foo, bar)
