# nitrocop-config: EnforcedStyle: single_statement_only

# Single-statement example: OK
it { is_expected.to be_good }

# Multi-statement but single-line: OK
it { is_expected.to be_good; is_expected.to be_nice }

# Single-statement multiline: OK
it do
  is_expected.to be_good
end

