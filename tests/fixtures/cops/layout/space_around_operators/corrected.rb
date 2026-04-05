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
adapter.properties = @props

# Ternary operator: missing space before :
incoming_page = resource.is_a?( Page ) ? resource : resource.to_page

# Ternary operator: missing space after ?
x = (name == '@') ? '' : name

# Ternary operator: missing space around both ? and : (nested)
lend = @rl_end > 0 ? @rl_end - ((@rl_editing_mode == @vi_mode) ? 1 : 0) : @rl_end

# Ternary operator: missing space in method argument context
target_url = target_url + (target_url.include?("?") ? "&" : "?") + params
