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
