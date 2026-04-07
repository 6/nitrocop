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
