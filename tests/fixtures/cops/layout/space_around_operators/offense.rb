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
adapter.properties =  @props
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
