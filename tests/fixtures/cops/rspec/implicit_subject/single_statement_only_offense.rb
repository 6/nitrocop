# nitrocop-config: EnforcedStyle: single_statement_only

# Multi-statement multiline example: offense
it do
  is_expected.to be_good
  ^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.
  is_expected.to be_nice
  ^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.
end
