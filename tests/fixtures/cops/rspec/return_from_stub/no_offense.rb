it do
  allow(Foo).to receive(:bar) { baz }
end
it do
  allow(Foo).to receive(:bar).and_return(42)
end
it do
  allow(Foo).to receive(:bar)
end
it do
  allow(Foo).to receive(:bar) { [42, baz] }
end
it do
  bar = 42
  allow(Foo).to receive(:bar) { bar }
end
# receive_message_chain with block is not flagged by RuboCop
it do
  allow(order).to receive_message_chain(:payments, :valid, :empty?) { false }
end
it do
  allow(obj).to receive_message_chain(:foo, :bar) { 42 }
end
# .freeze does not make a value static for ReturnFromStub
it do
  allow(example).to receive(:verb_for_action) { 'RefundPayment'.freeze }
end
it do
  allow(Foo).to receive(:bar) { "foo".freeze }
end
it do
  allow(Foo).to receive(:bar) { some_method.freeze }
end
# Block with parameter is dynamic (but only if body is dynamic)
it do
  allow(Foo).to receive(:bar) { |arg| arg }
end
# raise_error with block — not a stub, should not flag
it do
  expect { validate!({}) }.to raise_error(InvalidBlueprint) do
    'not a valid blueprint'
  end
end
it do
  expect { validate!(Integer) }.to raise_error(InvalidBlueprint) do
    'not a valid blueprint'
  end
end
