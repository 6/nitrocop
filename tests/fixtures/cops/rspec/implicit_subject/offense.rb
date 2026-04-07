it 'checks the subject' do
  is_expected.to be_good
  ^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.
end
it 'checks negation' do
  should be_good
  ^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.
end
it 'checks should_not' do
  should_not be_bad
  ^^^^^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.
end

it do should eq %Q{ifconfig vio0 2>&1 | awk -v s=down -F '[:<>,]' } +
      ^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.
                %Q{'NR == 1 && $3 == "UP" { s="up" }; /status:/ && $2 != " active" { s="down" }; END{ print s }'}
end

context 'after an its sibling' do
  its(:count) { is_expected.to eq 2 }
  its_map(['title']) { is_expected.to eq %w[Argentina Ukraine] }
                       ^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.
end

context 'with a symbol accessor after an its sibling' do
  its(:count) { is_expected.to be > 40 }
  its_map(:title) { is_expected.to include('Ukrainian hryvnia') }
                    ^^^^^^^^^^^ RSpec/ImplicitSubject: Don't use implicit subject.
end
