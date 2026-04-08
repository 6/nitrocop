# Irrelevant `and_return` calls are not RSpec stubs
it do
  library.visit.and_return(42)
end

# Multiple return values are treated as dynamic and ignored
it do
  allow(Foo).to receive(:bar).and_return(42, 43, 44)
end
