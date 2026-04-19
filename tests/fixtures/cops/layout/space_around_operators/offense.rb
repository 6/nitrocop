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

# Extra space before => in varied hash value contexts
MORE_HASH_ROCKETS = {
  'environ'  => proc do |response|
             ^^ Layout/SpaceAroundOperators: Operator `=>` should be surrounded by a single space.
    response
  end,
  :watermark  => [:watermark, :text],
              ^^ Layout/SpaceAroundOperators: Operator `=>` should be surrounded by a single space.
  :user   => User.first || User.create!,
          ^^ Layout/SpaceAroundOperators: Operator `=>` should be surrounded by a single space.
}

# Keyword operator: extra leading space before `or`
def fetch_modifier(key)
  MODIFIERS[key.to_sym]  or raise ArgumentError.new("Unknown modifier key: #{key}")
                         ^^ Layout/SpaceAroundOperators: Operator `or` should be surrounded by a single space.
end

# Keyword operator: extra leading space before `and` in modifier return
def redirect_back
  redirect_to :action => :show, :controller => :choices,
    :question_id => params[:question_id], :id => params[:id]  and return
                                                              ^^^ Layout/SpaceAroundOperators: Operator `and` should be surrounded by a single space.
end

# Keyword operator: extra leading space before `and` in a continued condition
if justNameCharacters(params["signature"]["firstnames"]) and
    justNameCharacters(params["signature"]["lastname"])   and
                                                          ^^^ Layout/SpaceAroundOperators: Operator `and` should be surrounded by a single space.
    municipalities.include?(params["signature"]["occupancy_county"]) and
    params["signature"]["vow"] == "1"
end

if justNameCharacters(params["first_names"]) and
    justNameCharacters(params["last_name"])   and
                                              ^^^ Layout/SpaceAroundOperators: Operator `and` should be surrounded by a single space.
    municipalities.include?(params["occupancy_county"])
end

# Comparison operators in modifier conditions must not align with later operators on neighbor lines
def finalize!
  @become              = false                    if @become              != true
                                                                          ^^ Layout/SpaceAroundOperators: Operator `!=` should be surrounded by a single space.
  @become_user         = nil                      if @become_user         == UNSET_VALUE
                                                                          ^^ Layout/SpaceAroundOperators: Operator `==` should be surrounded by a single space.
end

def retries_exceeded?
  max_retries  = Setting.get_with_default('queue.max_retries', 0).to_i
  max_retries  > 0 && self.retries >= max_retries
               ^ Layout/SpaceAroundOperators: Operator `>` should be surrounded by a single space.
end

hash['medline']  	= ui
^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.

s.name    	= 'rack-jekyll'
^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.

default['zabbix']['agent']['timeout']       	= '3'
^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.

@sessions << @drizzle_session    = proposals(:drizzle_session)
^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.

@sessions << @cloud_session      = proposals(:cloud_session)
^ Layout/SpaceAroundOperators: Operator `=` should be surrounded by a single space.
