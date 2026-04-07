# nitrocop-config: EnforcedStyle: single_statement_only

# Single-statement example: OK
it { is_expected.to be_good }

# Multi-statement but single-line: OK
it { is_expected.to be_good; is_expected.to be_nice }

# Single-statement multiline: OK
it do
  is_expected.to be_good
end

# FP fix: is_expected inside before block is NOT in an example context
before { is_expected.to be_ok }

# FP fix: is_expected inside let block
let(:result) { is_expected.to include "foo" }

# FP fix: is_expected inside a method def (not example context)
def check_result
  is_expected.to be_valid
end
