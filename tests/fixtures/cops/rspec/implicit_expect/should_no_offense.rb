# nitrocop-config: EnforcedStyle: should

# should style is correct — no offense
should be_truthy
should_not be_falsy

# Explicit expect is not flagged
expect(subject).to eq(42)
expect(something).not_to be_nil

# FP fix: multiline is_expected.to — RuboCop silently skips these
# (ENFORCED_REPLACEMENTS.fetch fails on whitespace in source range)
it do
  is_expected
    .to be_truthy
end

# FP fix: space between dot and method name
it { is_expected. to eq(1) }
