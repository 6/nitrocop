# Irrelevant `and_return` calls are not RSpec stubs
it do
  library.visit.and_return(42)
end

# Multiple return values are treated as dynamic and ignored
it do
  allow(Foo).to receive(:bar).and_return(42, 43, 44)
end

# Interpolated strings stay dynamic when the interpolation is non-static
it do
  bar = 42
  allow(service).to receive(:url).and_return("#{bar}/test-url")
end

# Parenthesized dynamic values stay dynamic
it do
  allow(Foo).to receive(:bar).and_return (value)
end
