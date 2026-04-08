# nitrocop-config: EnforcedStyle: single_statement_only

# Multi-statement multiline example: offense
it do
  is_expected.to be_good
  ^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.
  is_expected.to be_nice
  ^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.
end

# Parenthesized single-line example bodies are grouped, so RuboCop treats them
# as non-single-statement.
it { (is_expected.to belong_to(:interview_survey)) }
      ^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.
it { (is_expected.to belong_to(:interview_concept)) }
      ^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.

# Single-line examples with multiple statements are offenses too.
it { create(:configuration); should validate_uniqueness_of :name }
                             ^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.
specify { Footnotes::Filter.prefix = ''; should_not be_prefix }
                                         ^^^^^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.
