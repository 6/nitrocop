# nitrocop-config: EnforcedStyle: require_implicit

it { expect(subject).to be_good }
     ^^^^^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use explicit subject.
it do
  expect(subject).to be_good
  ^^^^^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use explicit subject.
end