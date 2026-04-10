# nitrocop-config: EnforcedStyle: empty_lines

# Block with empty lines at both beginning and end
group :development do

  gem 'spring'
  gem 'web-console'

end

# Single-statement block with empty lines
get '/' do

  send_file 'index.html'

end

# Brace block with empty lines
items.each { |x|

  puts x

}

# Single-line block (no offense regardless of style)
[1, 2, 3].each { |x| puts x }

# Empty block body (no offense with empty_lines style)
items.each do |x|
end

# Comment-only block body is treated as empty by RuboCop in empty_lines style
config.set_context do
  # Return a context object that gets evaluated within the controller
end

# Legacy colon syntax disables the whole Layout department in RuboCop
# so nested blocks inside the region are suppressed too.
# rubocop:disable Layout:LineLength
before do
  transport.tap do |double|
    allow(double).to receive(:get)
      .with('https://example.test/very/long/path')
      .and_return(:ok)
  end
end
# rubocop:enable Layout:LineLength
