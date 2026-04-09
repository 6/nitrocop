super do |klass, names, options|
          ^ Layout/SpaceAroundBlockParameters: No space before first block parameter detected.
                               ^ Layout/SpaceAroundBlockParameters: No space after last block parameter detected.
  puts klass
end

super { |x| puts x }
         ^ Layout/SpaceAroundBlockParameters: No space before first block parameter detected.

items.each { |x| puts x }
              ^ Layout/SpaceAroundBlockParameters: No space before first block parameter detected.

items.each { |a, b| puts a }
              ^ Layout/SpaceAroundBlockParameters: No space before first block parameter detected.
                  ^ Layout/SpaceAroundBlockParameters: No space after last block parameter detected.

shared_examples 'with extra space before multiline closing pipe' do |
  index:,
  other_index:
  |
^^ Layout/SpaceAroundBlockParameters: Extra space after last block parameter detected.
  puts index
end
