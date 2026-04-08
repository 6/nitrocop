# nitrocop-config: EnforcedStyle: empty_lines

# Block without empty lines at beginning or end
group :development do
  gem 'spring'
^ Layout/EmptyLinesAroundBlockBody: Empty line missing at block body beginning.
  gem 'web-console'
end
^ Layout/EmptyLinesAroundBlockBody: Empty line missing at block body end.

# Single-statement block without empty lines
get '/' do
  send_file 'index.html'
^ Layout/EmptyLinesAroundBlockBody: Empty line missing at block body beginning.
end
^ Layout/EmptyLinesAroundBlockBody: Empty line missing at block body end.

# Brace block without empty lines
items.each { |x|
  puts x
^ Layout/EmptyLinesAroundBlockBody: Empty line missing at block body beginning.
}
^ Layout/EmptyLinesAroundBlockBody: Empty line missing at block body end.

# Two-line brace block with closing delimiter on the body line still needs a blank beginning
items.each { |x|
  puts x }
^ Layout/EmptyLinesAroundBlockBody: Empty line missing at block body beginning.

# Arrow lambda brace block with closing delimiter on the body line still needs a blank beginning
handler = -> (purchase) {
  purchase.id }
^ Layout/EmptyLinesAroundBlockBody: Empty line missing at block body beginning.

# Two-line do/end block with closing delimiter on the body line still needs a blank beginning
items.each do |x|
  puts x end
^ Layout/EmptyLinesAroundBlockBody: Empty line missing at block body beginning.

# Two-line brace block with body starting on the opening line still needs a blank beginning
it { is_expected.to validate_uniqueness_of(:github_url).
  case_insensitive.with_message('Project has already been suggested.') }
^ Layout/EmptyLinesAroundBlockBody: Empty line missing at block body beginning.

# Forwarding super blocks also need blank lines in empty_lines style
def method_missing(*, &block)
  super do |klass, names, options|
    options = add_option_in_place_of_name(klass, options)
^ Layout/EmptyLinesAroundBlockBody: Empty line missing at block body beginning.

    validate!(names)
  end
^ Layout/EmptyLinesAroundBlockBody: Empty line missing at block body end.
end
