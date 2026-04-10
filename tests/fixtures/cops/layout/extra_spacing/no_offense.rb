x = 1
y = 2
foo(1, 2)
bar = "hello world"
name      = "RuboCop"
website   = "rubocop.org"
object.method(arg) # this is a comment

# Aligned assignment operators (AllowForAlignment: true)
a   = 1
b   = 2

# Alignment across blank lines
a  = 1

b  = 2

# Alignment across comment-only lines
name    = "one"
# this is a comment
website = "two"

# Aligned trailing comments
x = 1 # first comment
y = 2 # second comment

# Multiline hash (spacing handled by Layout/HashAlignment, not ExtraSpacing)
config = {
  name:      "RuboCop",
  website:   "rubocop.org",
  version:   "1.0"
}

# Compound assignment alignment (e.g. += aligns with =)
retries     += 1
@http_client = http_client

# Whitespace at the beginning of the line (indentation)
  m = "hello"

# Whitespace inside a string
m = "hello   this"

# Trailing whitespace (handled by Layout/TrailingWhitespace, not here)
class Benchmarker < Performer
end

# Aligned values of an implicit hash literal (multiline)
register(street1:    '1 Market',
         street2:    '#200',
         :city =>    'Some Town',
         state:      'CA')

# Space between key and value in a hash with hash rockets (multiline)
ospf_h = {
  'ospfTest'    => {
    'foo'      => {
      area: '0.0.0.0', cost: 10, hello: 30, pass: true },
    'longname' => {
      area: '1.1.1.38', pass: false },
    'vlan101'  => {
      area: '2.2.2.101', cost: 5, hello: 20, pass: true }
  }
}

# Lining up assignments with empty lines and comments in between
# (allowed with AllowForAlignment: true)
a   += 1

# Comment
aa   = 2
bb   = 3

a  ||= 1

# Lining up different kinds of assignments
type_name ||= value.class.name if value
type_name   = type_name.to_s   if type_name

# Aligned trailing comments (same column)
one  # comment one
two  # comment two

# Only one space before comment is fine (no extra spacing)
object.method(argument) # this is a comment

# Token alignment: same operator at same column across lines
y, m = (year * 12 + (mon - 1) + n).divmod(12)
m,   = (m + 1)                    .divmod(1)

# Aligned values in array of hashes: commas at same columns
items = [
  {id: 1, name: 'short'  , code: 'equals'      },
  {id: 2, name: 'longer' , code: 'greater_than'},
  {id: 3, name: 'longest', code: 'less_than'   },
]

# Aligned method calls with commas
has_many :items  , dependent: :destroy
has_many :images , dependent: :destroy
has_many :options, dependent: :destroy

# Aligned trailing comments separated by blank lines
unless nochdir
  Dir.chdir "/"    # Release old working directory.
end

File.umask 0000    # Ensure sensible umask.

# Extra spaces inside %w() word arrays are separators, not extra spacing
builtins = %w(
  foo  bar  baz
  one  two  three
)
trailing = %w(foo bar  )

# Extra spaces inside %i() symbol arrays
syms = %i(foo  bar  baz)

# Extra spaces inside %W() and %I() arrays
words = %W(hello  world  #{name})
isyms = %I(hello  world)

# Single tab between tokens is not extra spacing (1 whitespace char)
data = ['ADJ',	'Adjective']
x =	1
when 0b0001	then process
fill_in 'field',	with: value

# Backslash line continuation — spacing before \ is not flagged
expected =  \
  "Real HTTP connections are disabled"
message = "The platform"     \
  "(#{platform}) is not compatible"

# Aligned values with multibyte characters (CJK)
# Commas should align visually even though byte offsets differ
data = [
  {id: 1, name: 'short'     , code: 'a'},
  {id: 2, name: 'longer'    , code: 'b'},
]

# Assignment = aligned with << on adjacent line (AllowForAlignment: true)
# RuboCop treats << as an assignment-like operator that can align with =
pages  = pages.values
pages << page_buffer

# Variable with = aligned with << (append) on next line
hdr  = "<head><style>"
hdr << "@page{size: landscape}"

# Multiple aligned = and << operators
message  = "Widget Generation..."
message << " (error)" if error
message << " (timeout)" if timeout

# Aligned = and << with same-indent search
id  = inputs ? inputs.sort_by { |k, _| k }.hash.to_s : ''
id << ':'

# Three-line alignment: =, <<, and = again
e.document     = @document
@current_node << e
@current_node  = e

# Aligned = and << with longer variable names
results   = [set_to_array(statement.getResultSet)]
results  << set_to_array(statement.getResultSet) while statement.getMoreResults

# Compound assignment aligned with <<
columns  = ((options && options[:columns]) || self.class.column_names_symbols.dup)
columns << :id

# Chained `.to` / `.not_to` with same dot column — alignment is intentional
# RuboCop's tokenizer makes `.` a separate token, so `.` == `.` is a Mode 2 match.
expect(clo)  .to be_within(0.1).of(1.0)
expect(clo)  .not_to be_nil

# Heredoc openers can be followed by a same-line block close
let(:hiera_config) { <<~CONF  }
---
version: 5
CONF

# Trailing content after heredoc opener is not checked (RuboCop's token
# stream jumps to the heredoc body, so the trailing text is never paired)
STATS = <<-STATS  # :nodoc:
body
STATS

fail <<~EOM  if condition
  body
EOM

code = <<~__CODE     # insert the code update
  body
__CODE

# Heredoc interpolation lines starting with #{ are not comments
# Adjacent #{} lines can have alignment
<<~UNIT_CONFIG_FILE
  #{unit_settings.compact.map { |key, value| "#{key}=#{value}" }.join("\n")}
  #{environment.compact.map   { |key, value| "Environment=#{key}=#{value}" }.join("\n")}
UNIT_CONFIG_FILE

# Alignment across commented-out #{...} lines (they ARE comments, not interpolation)
# RuboCop's alignment search skips these lines and finds real code alignment.
functions = [
  {:name => :each,             :args => '[1,2,3]'},
  {:name => :epp,              :args => '"template"'},
  {:name => :filter,           :args => '[4,5,6]'},
  # find_file() called by binary_file
  #{:name => :find_file,           :args => '[4,5,6]'},
  {:name => :inline_epp,       :args => '"test"'},
  #{:name => :lest,             :args => '100'},
  {:name => :map,              :args => '[7,8,9]'},
]

# Interpolated string alignment: = aligned with = on adjacent line
# RuboCop's tokenizer splits interpolated strings into tSTRING_BEG + contents,
# so the opening " is a separate token that matches the closing " of "" above.
aws_sqs_queue_url = ""
aws_sqs_queue_url =  "https://sqs.#{url}.amazonaws.com" if url
