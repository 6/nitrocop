good_variable = 1
x = 2
my_value = 3
_unused = 4
first_name = "John"
@good_var = 1
@@good_var = 2
$good_var = 3
$_ = "discard"
$0
$1
$!
@_unused = nil
def foo(good_param, name:, _unused:)
end
$MY_GLOBAL = 0
$globalVar = "test"
$CamelCaseGlobal = true
$HI = 0
$LO = 0
good_var = 1
do_something(good_var)
items.each { |item| item }
good_name, ok = [1, 2]
good_compound ||= 1
@good_ivar ||= compute
@@good_cvar += 1
$GOOD_GLOBAL ||= 0

# Pattern matching destructuring - RuboCop skips match_var nodes
@params => {
  textDocument: { uri: },
  position: pos,
  newName:,
}

case data
in { camelKey: }
  nil
end

# Regex named captures - RuboCop skips match_with_lvasgn nodes
/(?<channelClaim>\w+)/ =~ params

# Unicode lowercase letters are valid snake_case (matches Ruby [[:lower:]])
héllo = 1
µ_value = 2

# rescue => var with snake_case should not be flagged
begin
  something
rescue => good_error
  nil
end

begin
  something
rescue StandardError => @good_ivar
  nil
end

# Destructured block and method params with valid snake_case names
items.each do |ok, (inner_a, inner_b)|
end

def bar((first_val, second_val)); end

# Post-rest parameters
[1].each do |*_, post_param|
end
