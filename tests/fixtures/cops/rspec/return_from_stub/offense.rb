it do
  allow(Foo).to receive(:bar) { 42 }
                              ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
it do
  allow(Foo).to receive(:baz) {}
                              ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
it do
  allow(Foo).to receive(:qux) { [1, 2] }
                              ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
# Constants are static values (recursive_literal_or_const?)
it do
  allow(Foo).to receive(:bar) { SomeConstant }
                              ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
it do
  allow(Foo).to receive(:bar) { Module::CONSTANT }
                              ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
it do
  allow(Foo).to receive(:bar) { {Life::MEANING => 42} }
                              ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
# Ranges are static values
it do
  allow(Foo).to receive(:bar) { 1..10 }
                              ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
# Regexps are static values
it do
  allow(Foo).to receive(:bar) { /pattern/ }
                              ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
# Rational and imaginary literals are static values
it do
  allow(Foo).to receive(:bar) { 1r }
                              ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
it do
  allow(Foo).to receive(:bar) { 1i }
                              ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
# Block on chained .with(...) — block binds to .with, not to .to
it do
  allow(Question).to receive(:meaning).with(:universe) { 42 }
                                                       ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
# Block on chained .once — block binds to .once, not to .to
it do
  expect(Foo).to receive(:bar).once { 42 }
                                    ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
# Block on chained .with and static array — block binds to .with
it do
  allow(Foo).to receive(:bar).with(1, 2) { [1, 2] }
                                         ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
# allow_any_instance_of / expect_any_instance_of are also stub setup methods
it do
  allow_any_instance_of(Foo).to receive(:bar) { 42 }
                                              ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
it do
  expect_any_instance_of(Foo).to receive(:baz) { true }
                                               ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
it do
  allow_any_instance_of(Foo).to receive(:qux).with(:arg) { "hello" }
                                                         ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
# is_expected is equivalent to expect(subject)
it do
  is_expected.to receive(:can?) { true }
                                ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
it do
  is_expected.to receive(:can?).with(:read, 123) { true }
                                                 ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
# Block with parameters but static body — RuboCop still flags
it do
  allow_any_instance_of(Foo).to receive(:load).with("file", any_args) do |config, name|
                                                                      ^^ RSpec/ReturnFromStub: Use `and_return` for static values.
    nil
  end
end
it do
  allow_any_instance_of(Foo).to receive(:find) {|path| nil}
                                               ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
# __FILE__ is a static pseudo-literal
it do
  allow(RSpec.configuration).to receive(:loaded_spec_files) { [__FILE__] }
                                                            ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
# Arbitrary matcher receivers like `wrapped.to receive(...)` are still stub setup
it do
  allow_it.to receive(:results) { "results" }
                                ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
it do
  allow_it.to receive(:results) { :all }
                                ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
it do
  wrapped.to receive(:foo) do
                           ^^ RSpec/ReturnFromStub: Use `and_return` for static values.
    4
  end
end
it do
  wrapped.to receive(:foo) do
                           ^^ RSpec/ReturnFromStub: Use `and_return` for static values.
    4
  end.and_return(6)
end
it do
  wrapped.to receive(:foo) { 5 }
                           ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
it do
  wrapped.to receive(:foo) { :curly } do
                           ^ RSpec/ReturnFromStub: Use `and_return` for static values.
    :do_end
  end
end
it do
  wrapped.to receive(:foo).with(1) { :a }
                                   ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
it do
  wrapped.to receive("foo").with(2) { :b }
                                    ^ RSpec/ReturnFromStub: Use `and_return` for static values.
end
it do
  wrapped.to receive(:foo).with(1) do
                                   ^^ RSpec/ReturnFromStub: Use `and_return` for static values.
    :a
  end
end
it do
  wrapped.to receive(:foo).with(1) { :curly } do
                                   ^ RSpec/ReturnFromStub: Use `and_return` for static values.
    :do_end
  end
end

# Backtick command literals are still static in a multi-statement block body
it do
  allow(driver).to receive(:`) do |cmd|
                               ^^ RSpec/ReturnFromStub: Use `and_return` for static values.
    `false`
    "Error: Something went wrong"
  end
end
