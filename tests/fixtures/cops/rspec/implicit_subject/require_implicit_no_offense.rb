# nitrocop-config: EnforcedStyle: require_implicit

it { is_expected.to be_good }
it { should be_good }
it { should_not be_bad }
it do
  is_expected.to be_good
end

subject(:instance) { described_class.new }
it { expect(instance).to be_good }
it { expect { subject }.to change(goodness, :count) }
its(:quality) { is_expected.to be(:high) }