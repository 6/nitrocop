# nitrocop-config: EnforcedStyle: named_only

# `subject(:name, &builder)` is not a subject definition for named_only.
# RuboCop sees a block-pass send, not a `subject { ... }` block node.
RSpec.describe 'Concurrent::Actor' do
  describe 'spawning' do
    describe 'Actor#spawn!' do
      subjects = { spawn: -> { Actor.spawn!(Object, :ping, 'arg') } }

      subjects.each do |desc, subject_definition|
        describe desc do
          subject(:actor, &subject_definition)

          it('executor should be global') do
            expect(subject.executor).to eq Concurrent.global_io_executor
          end

          it 'returns arg' do
            expect(subject.ask!(:anything)).to eq 'arg'
          end
        end
      end
    end
  end
end
