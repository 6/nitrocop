it 'is good' do
  expect(subject).to be_good
end
specify 'is good' do
  expect(subject).to be_good
end
it { expect(subject).to be_good }
specify { expect(subject).to be_good }
it { is_expected.to be_truthy }
it do
  expect(subject).to be_good
end
# RuboCop always allows multiline `specify` without description
specify do
  result = service.call
  expect(result).to be(true)
end

it '', :aggregate_failures do
  expect(subject).to be_good
end

# Block argument — not a block body, should not be flagged
it(&block)

# New example methods used correctly (no description is fine under always_allow)
scenario { visit root_path }
xit { expect(subject).to be_good }
pending { expect(subject).to be_good }
focus { expect(subject).to be_good }
skip { expect(subject).to be_good }
fit { expect(subject).to be_good }
