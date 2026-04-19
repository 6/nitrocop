x = 1
x == ""
x = 1
x != y
a => "hello"
x + y
x - y
x * y
x && y
x || y
x && y

# Compound assignment operators
x += 0
y -= 0
z *= 2
x ||= 0
y &&= 0

# Match operators
x =~ /abc/
y !~ /abc/

# Class inheritance
class Foo < Bar
end

# Singleton class
class << self
end

# Rescue =>
begin
rescue Exception => e
end

# Triple equals
Hash === z

# Exponent with spaces (default no_space style should flag)
x = a * b**2

# Setter call without spaces
x.y = 2

# Extra spaces around = with subsequent assignment at different column
x = 1
y = 2

# Extra spaces around => (not aligned)
{'key' => 'val'}

'arrow' => [:arrow, :down],

@_apipie_dsl_data = {

result = [{

html_block = if render_partial?

message[:bcc] = 'mikel@bcc.lindsaar.net'
message[:cc] = 'mikel@cc.lindsaar.net'

# Extra space before << is not valid alignment with = on neighbor line
t.pattern = 'spec/**/*_spec.rb'
t.libs << 'spec'
t.warning = false

# Extra space before => inside hash (not aligned)
{ 'environ' => 1 }

# Extra space before => with symbol key
{ :reset => "\e[0m" }

# Setter call with extra trailing space (not aligned with neighbor)
obj.prop = @v

# Ternary operator: missing space before :
incoming_page = resource.is_a?( Page ) ? resource : resource.to_page

# Ternary operator: missing space after ?
x = (name == '@') ? '' : name

# Ternary operator: missing space around both ? and : (nested)
lend = @rl_end > 0 ? @rl_end - ((@rl_editing_mode == @vi_mode) ? 1 : 0) : @rl_end

# Ternary operator: missing space in method argument context
target_url = target_url + (target_url.include?("?") ? "&" : "?") + params

# Extra leading space before = when subsequent assignment exists but is NOT aligned
# (non-assignment lines between them don't break the search)
workload = []
foo(bar)
tag = Tag.create name: 'tag 1'
baz(qux)
options = nil

# Extra space before => with a block value
FILE_SIGNATURES = {
  'environ' => proc do |response|
    next if !response.body.include?('DOCUMENT_ROOT=')
  end,
}

# Extra leading space before = after a blank line in an assignment group
note_1 = Note.create name: 'test', content: 'dummy content'
note_2 = Note.create name: 'test 2', content: 'dummy content'

tag = Tag.create name: 'tag 1', note_id: note_1.id
Tag.create name: 'tag 2'
@json_expected = '{}'

# Extra leading space before = must not align across blank-line-separated groups
vm1 = FactoryBot.create(:vm_or_template)
@vm_perf1 = FactoryBot.create(:vim_performance_state, :resource => vm1)
vm1.delete

@vm_perf2 = FactoryBot.create(:vim_performance_state, :resource => FactoryBot.create(:vm_or_template))

ems1 = FactoryBot.create(:ext_management_system)
@ems_perf1 = FactoryBot.create(:vim_performance_state, :resource => ems1)

# Extra leading space before = in chained assignments on the same line
temp_stdin  = Tempfile.new 'temp_stdin'
temp_stdout = Tempfile.new 'temp_stdout'

Reline.input  = @input = File.open(temp_stdin.path, 'w+')
Reline.output = @output = File.open(temp_stdout.path, 'w+')

# Only the first = on each neighbor line participates in assignment alignment
SetUIDBit = ReadBit = 4
SetGIDBit = WriteBit = 2
StickyBit = ExecBit  = 1

# Extra space before => in varied hash value contexts
MORE_HASH_ROCKETS = {
  'environ' => proc do |response|
    response
  end,
  :watermark => [:watermark, :text],
  :user => User.first || User.create!,
}

# Keyword operator: extra leading space before `or`
def fetch_modifier(key)
  MODIFIERS[key.to_sym] or raise ArgumentError.new("Unknown modifier key: #{key}")
end

# Keyword operator: extra leading space before `and` in modifier return
def redirect_back
  redirect_to :action => :show, :controller => :choices,
    :question_id => params[:question_id], :id => params[:id] and return
end

# Keyword operator: extra leading space before `and` in a continued condition
if justNameCharacters(params["signature"]["firstnames"]) and
    justNameCharacters(params["signature"]["lastname"]) and
    municipalities.include?(params["signature"]["occupancy_county"]) and
    params["signature"]["vow"] == "1"
end

if justNameCharacters(params["first_names"]) and
    justNameCharacters(params["last_name"]) and
    municipalities.include?(params["occupancy_county"])
end

# Comparison operators in modifier conditions must not align with later operators on neighbor lines
def finalize!
  @become              = false                    if @become != true
  @become_user         = nil                      if @become_user == UNSET_VALUE
end

def retries_exceeded?
  max_retries  = Setting.get_with_default('queue.max_retries', 0).to_i
  max_retries > 0 && self.retries >= max_retries
end

hash['medline'] = ui

s.name = 'rack-jekyll'

default['zabbix']['agent']['timeout'] = '3'

@sessions << @drizzle_session = proposals(:drizzle_session)
@sessions << @postgresql_session = proposals(:postgresql_session)

@sessions << @cloud_session = proposals(:cloud_session)
@sessions << @business_session   = proposals(:business_session)
