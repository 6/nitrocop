# nitrocop-config: EnforcedStyle: same_line

# Last hash element is an array containing a heredoc.
# RuboCop skips because the heredoc terminator reaches the last element line.
tests = {
  test_case: [<<~RUBY, 42]
    value
  RUBY
}

# Legacy single-colon directive syntax disables the whole Layout department.
# RuboCop suppresses Layout/MultilineHashBraceLayout here.
# rubocop:disable Layout:LineLength
config = {
  a: 1,
  b: 2
}
# rubocop:enable Layout:LineLength
