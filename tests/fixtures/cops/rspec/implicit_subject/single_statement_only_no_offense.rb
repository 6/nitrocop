# nitrocop-config: EnforcedStyle: single_statement_only

# Single-statement example: OK
it { is_expected.to be_good }

# Single-statement multiline: OK
it do
  is_expected.to be_good
end
