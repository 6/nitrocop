# nitrocop-config: EnforcedStyle: require_implicit

it { expect(subject).to be_good }
     ^^^^^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use explicit subject.
it do
  expect(subject).to be_good
  ^^^^^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use explicit subject.
end

# RuboCop flags expect(subject) in any context, not just example blocks
before { expect(subject).to be_valid }
         ^^^^^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use explicit subject.
let(:result) { expect(subject).to include "foo" }
               ^^^^^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use explicit subject.
describe 'something' do
  expect(subject).to be_ok
  ^^^^^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use explicit subject.
end