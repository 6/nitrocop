x =1
  ^ Layout/SpaceAroundOperators: Surrounding space missing for operator `=`.
x ==""
  ^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `==`.
x= 1
 ^ Layout/SpaceAroundOperators: Surrounding space missing for operator `=`.
x!= y
 ^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `!=`.
a =>"hello"
  ^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `=>`.
x +y
  ^ Layout/SpaceAroundOperators: Surrounding space missing for operator `+`.
x- y
 ^ Layout/SpaceAroundOperators: Surrounding space missing for operator `-`.
x *y
  ^ Layout/SpaceAroundOperators: Surrounding space missing for operator `*`.
x &&y
  ^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `&&`.
x ||y
  ^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `||`.
x  && y
   ^^ Layout/SpaceAroundOperators: Operator `&&` should be surrounded by a single space.

# Compound assignment operators
x +=0
  ^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `+=`.
y -=0
  ^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `-=`.
z *=2
  ^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `*=`.
x ||=0
  ^^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `||=`.
y &&=0
  ^^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `&&=`.

# Match operators
x =~/abc/
  ^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `=~`.
y !~/abc/
  ^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `!~`.

# Class inheritance
class Foo<Bar
         ^ Layout/SpaceAroundOperators: Surrounding space missing for operator `<`.
end

# Singleton class
class<<self
     ^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `<<`.
end

# Rescue =>
begin
rescue Exception=>e
                ^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `=>`.
end

# Triple equals
Hash===z
    ^^^ Layout/SpaceAroundOperators: Surrounding space missing for operator `===`.

# Exponent with spaces (default no_space style should flag)
x = a * b ** 2
          ^^ Layout/SpaceAroundOperators: Space around operator `**` detected.

# Setter call without spaces
x.y =2
    ^ Layout/SpaceAroundOperators: Surrounding space missing for operator `=`.

# Extra spaces around = with subsequent assignment at different column
x  = 1
   ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.
y = 2

# Extra spaces around => (not aligned)
{'key'  => 'val'}
        ^^ Layout/SpaceAroundOperators: Operator `=>` should be surrounded by a single space.

'arrow'               => [:arrow, :down],
                      ^^ Layout/SpaceAroundOperators: Operator `=>` should be surrounded by a single space.

@_apipie_dsl_data =  {
                  ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.

result =  [{
       ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.

html_block =    if render_partial?
           ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.

message[:bcc] =           'mikel@bcc.lindsaar.net'
              ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.
message[:cc] =            'mikel@cc.lindsaar.net'
             ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.

# Extra space before << is not valid alignment with = on neighbor line
t.pattern = 'spec/**/*_spec.rb'
t.libs    << 'spec'
          ^^ Layout/SpaceAroundOperators: Operator `<<` should be surrounded by a single space.
t.warning = false

# Extra space before => inside hash (not aligned)
{ 'environ'  => 1 }
             ^^ Layout/SpaceAroundOperators: Operator `=>` should be surrounded by a single space.

# Extra space before => with symbol key
{ :reset   => "\e[0m" }
           ^^ Layout/SpaceAroundOperators: Operator `=>` should be surrounded by a single space.

# Setter call with extra trailing space (not aligned with neighbor)
obj.prop =  @v
         ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.

# Ternary operator: missing space before :
incoming_page = resource.is_a?( Page ) ? resource: resource.to_page
                                                 ^ Layout/SpaceAroundOperators: Surrounding space missing for operator `:`.

# Ternary operator: missing space after ?
x = (name == '@')? '' : name
                 ^ Layout/SpaceAroundOperators: Surrounding space missing for operator `?`.

# Ternary operator: missing space around both ? and : (nested)
lend = @rl_end > 0 ? @rl_end - ((@rl_editing_mode == @vi_mode)?1:0) : @rl_end
                                                              ^ Layout/SpaceAroundOperators: Surrounding space missing for operator `?`.
                                                                ^ Layout/SpaceAroundOperators: Surrounding space missing for operator `:`.

# Ternary operator: missing space in method argument context
target_url = target_url + (target_url.include?("?")?"&":"?") + params
                                                   ^ Layout/SpaceAroundOperators: Surrounding space missing for operator `?`.
                                                       ^ Layout/SpaceAroundOperators: Surrounding space missing for operator `:`.

# Extra leading space before = when subsequent assignment exists but is NOT aligned
# (non-assignment lines between them don't break the search)
workload  = []
          ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.
foo(bar)
tag    = Tag.create name: 'tag 1'
       ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.
baz(qux)
options = nil

# Extra space before => with a block value
FILE_SIGNATURES = {
  'environ'  => proc do |response|
             ^^ Layout/SpaceAroundOperators: Operator `=>` should be surrounded by a single space.
    next if !response.body.include?('DOCUMENT_ROOT=')
  end,
}

# Extra leading space before = after a blank line in an assignment group
note_1 = Note.create name: 'test', content: 'dummy content'
note_2 = Note.create name: 'test 2', content: 'dummy content'

tag    = Tag.create name: 'tag 1', note_id: note_1.id
       ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.
Tag.create name: 'tag 2'
@json_expected = '{}'

# Extra leading space before = must not align across blank-line-separated groups
vm1      = FactoryBot.create(:vm_or_template)
         ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.
@vm_perf1 = FactoryBot.create(:vim_performance_state, :resource => vm1)
vm1.delete

@vm_perf2 = FactoryBot.create(:vim_performance_state, :resource => FactoryBot.create(:vm_or_template))

ems1      = FactoryBot.create(:ext_management_system)
          ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.
@ems_perf1 = FactoryBot.create(:vim_performance_state, :resource => ems1)

# Extra leading space before = in chained assignments on the same line
temp_stdin  = Tempfile.new 'temp_stdin'
temp_stdout = Tempfile.new 'temp_stdout'

Reline.input  = @input  = File.open(temp_stdin.path, 'w+')
                        ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.
Reline.output = @output = File.open(temp_stdout.path, 'w+')

# Only the first = on each neighbor line participates in assignment alignment
SetUIDBit = ReadBit  = 4
                     ^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.
SetGIDBit = WriteBit = 2
StickyBit = ExecBit  = 1
