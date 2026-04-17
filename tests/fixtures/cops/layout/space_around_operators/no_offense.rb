x = 1
x == ""
x != y
a => "hello"
{a: 1, b: 2}
x += 1
"hello=world"
# x=1 inside comment
x = "a==b"

# Default parameters (handled by SpaceAroundEqualsInParameterDefault)
def foo(bar=1)
end
def baz(x=1, y=2)
end

# Spaceship operator (<=>) should not trigger => check
x <=> y
[1, 2, 3].sort { |a, b| a <=> b }

# Operator method definitions should not be flagged
def ==(other)
  id == other.id
end

def !=(other)
  !(self == other)
end

def []=(key, value)
  @data[key] = value
end

def <=>(other)
  name <=> other.name
end

def self.===(other)
  other.is_a?(self)
end

def >=(other)
  value >= other.value
end

# Safe navigation with operator method: &.!=
table_name&.!= node.left.relation.name

# Method call with dot before operator
x.== y

# Binary operators with proper spacing
x + y
x - y
x * y
x / y
x % y
x & y
x | y
x ^ y
x << y
x >> 1
x && y
x || y
x < y
x > y
x <= y
x >= y
x <=> y

# Unary operators (not binary — should not be flagged)
z = -x
z = +x

# Exponent operator with no_space style (default) should not be flagged
x = 2**10
y = n**(k - 1)

# AllowForAlignment: operators aligned across adjacent lines
title  = data[:title]  || ''
url    = data[:url]    || ''
width  = data[:width]  || 0
height = data[:height] || 0

# Trailing spaces before comment after operator — not flagged
x ||  # fallback
  y
a &&  # condition check
  b

# Operator at start of line (continuation) — indentation, not extra spacing
result = foo \
  + bar
x = a \
    || b

# Operator at start of line with no indentation is also accepted
a = 1 + 1 \
+ 1
"a"
-
"b"
%
?\C-a
%
?\M-a

# Compound assignments with proper spacing
x += 1
y -= 2
z *= 3
a /= 4
b %= 5
c ||= 0
d &&= true
e **= 2
f <<= 1
g >>= 1
h ^= 0xff
i |= 0x01
j &= 0xff

# Standalone ||= / &&= with extra leading space are accepted
@config  ||= File.open(config_path) { |yf| YAML::load(yf) }

self.state  ||= nil

feature_flag  &&= true

# Match operators with proper spacing
x =~ /abc/
y !~ /abc/

# Class inheritance with proper spacing
class Foo < Bar
end

# Singleton class with proper spacing
class << self
end

# Rescue => with proper spacing
begin
rescue Exception => e
end

# Triple equals with proper spacing
Hash === z

# Setter call with proper spacing
x.y = 2

# Ternary operator with proper spacing
x == 0 ? 1 : 2
result = condition ? true_val : false_val
nested = a ? (b ? c : d) : e

# Rational literal (no_space style default for /)
x = 2/3r

# Ranges should not be flagged
a, b = (1..2), (1...3)

# Scope operator should not be flagged
Zlib::GzipWriter

# Operator symbols should not be flagged
func(:-)

# Tabs around operator are acceptable
a =	1
x	= 1
y	=	2
'000'	=>	'General error'
'001' =>	'3D Not authenticated'
x ==	y
x	!= y

# Cross-operator alignment: ||= aligned with = (same end column)
PATH_PATTERN           = /^\/\w+/
PROTOCOL_PATTERN       = /^\w+:\/\//
README                 = File.dirname(__FILE__) + '/../../README.md'
@output              ||= STDOUT

# Cross-operator alignment: += aligned with = (same end column)
x  = 1
y += 2

# Cross-operator alignment: various compound operators aligned
found        += items
total        += count
status      ||= 0

# Trailing spaces after = are allowed when the right-hand sides align
email =  "Susan_foo@gmail.com"
password = "Susan_foo"

# Hash with multi-byte UTF-8 keys aligned by => (curly quotes are 3 bytes each)
# Must not flag any of these as "extra space" around =>
rewrites = {
  'should amass debt'                    => 'amasses debt',
  'should echo the input'                => 'echoes the input',
  "shouldn\u2019t return something"      => 'does not return something',
  "SHOULDN\u2019T BE true"               => 'IS NOT true',
}

# Multi-line hash pairs may keep extra space after => when the pairs align
params = {
  'GroupingSid' =>  Twilio.serialize_list(grouping_sid) { |e| e },
  'DateCreatedAfter' =>  Twilio.serialize_iso8601_datetime(date_created_after),
  'DateCreatedBefore' =>  Twilio.serialize_iso8601_datetime(date_created_before),
}

# Extra leading space before = on standalone assignments (no subsequent assignment neighbor)
# RuboCop does not flag these because there's no subsequent assignment to misalign with
x  = 1

projects  = 3.times.map { |i| i }

# Extra leading space before = at end of assignment group (no subsequent assignment)
@start_token = "["
@end_token   = "]"
@assignment  = "="
@statement_terminator  = ";"

# Plain = followed by non-assignment line with = only inside a string
rel  = '/test'
expect(foo).to eq("test?name2=val2")

# Setter calls with aligned trailing space after = (RHS values align)
# RuboCop does not flag these because aligned_with_something? returns true
cors_rule.allowed_origins =  foo
cors_rule.allowed_methods =  bar
cors_rule.max_age_in_seconds = baz

# Aligned values where exact token matches across lines (e.g., 1 aligns with 1 in -1)
FTO   =  1
BTO   = -1
FFIND =  2
BFIND = -2

# Explicit method call for []=  should not be flagged (not an operator)
@flows.[]=(*args)

# Aligned trailing space after == (RHS token matches on adjacent line)
return { "json_class" => Float, "raw" => "Infinity" }  if self.infinite? ==  1
return { "json_class" => Float, "raw" => "-Infinity" } if self.infinite? == -1
return { "json_class" => Float, "raw" => "NaN" }       if self.nan?

# Endless method definitions — the `=` is def syntax, not an assignment operator
def key?(key)       = @attributes.key?(key)
alias include? key?
def size            = @attributes.size
alias length size
def empty?          = @attributes.empty?

# Extra leading space before = aligned with PRECEDING assignment (non-assignment lines between)
options   = foo
quux()
new_line  = true

# Mixed tab + spaces after a tab-aligned identifier are accepted
Base32	     = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"
Alpha	     = UpperAlpha + LowerAlpha

# Comparison operators do not count as assignment neighbors
loop do
  return false if Time.now >= timeoutTime
  data, inetAddr  = @ClientSocket.recvfrom_nonblock(READ_SIZE)
  break
end

# Extra spaces after an operator are accepted when the line ends there
foo +            
  bar

# Standalone nested assignment is accepted when there is no later assignment group
SetUIDBit = ReadBit  = 4
