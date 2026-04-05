set_app("RuboCop")
website = "https://github.com/rubocop"

x = 1

method_call(arg1, arg2)

# Alignment where adjacent token is NOT preceded by space (coincidental vertical alignment)
d_is_vertically_aligned do
  _______________________d
end

# Extra space before a float in multiline array
{:a => "a",
 :b => [nil, 2.5]}

# Extra spacing in class inheritance
class A < String
end

# Extra spacing before a unary plus in an argument list
assert_difference(MyModel.count, +2,
                  3, +3,
                  4,+4)

# Single-line hash with extra spaces
hash = {a: 1, b: 2}

# Trailing comments at different columns - NOT aligned, should be flagged
check_a_pattern_result # comment A
check_b # comment B
check_c_patterns # comment C

# Extra spaces inside empty word arrays (RuboCop flags these)
a = %w( )

# Extra space after assert (not aligned with anything meaningful)
assert @fake_stderr.contained?(/flag/)
assert !@called

# Extra space after opening brace
{ portal: {
  name: 'test_portal'
} }

# Alignment FN: ||= with extra spaces not aligned with adjacent =
# (different last_column of = sign)
@signatures[pair_hash] ||= {}
@data_gathering[pair_hash] ||= {}

let(:output_missing) { <<-EOT
EOT
}

option. #{ BlueHydra.config["file"] ? "\n\nReading data from " + BlueHydra.config["file"]  + '.' : '' }

assert { case1("@gptあ") == "あ" }

[0, 0] => [:posixclass, :word, PosixClass, name: 'word', text: '[:word:]']

text str: 'The Title', layout: :title # from custom-layout.yml

expected_out = Torch.tensor([
  [[ 0.7493,  0.4482, -2.1426, 0.5586,  0.5540, -0.1676],
   [-1.7787,  1.3332, -0.3269, -0.2184,  0.9501,  0.0408]],

  [[ 0.0258, -0.3633, 0.4725, -0.5102,  1.8175, -1.4423],
   [-0.8428,  0.8163, -1.7820, 0.9993,  0.1579,  0.6513]],
])

html = <<-EOF
#{foo(1, 2)}
#{bar(3, 4)}
#{baz(5, 6)}
EOF

(%w[ id lock_version position version_comment created_at updated_at created_by_id updated_by_id type original_record_id])

def builtin_state
  raise Bud::Error unless @tables.empty?

  loopback :localtick, [:col1]
  @stdio = terminal :stdio
  scratch :halt, [:key]
  @periodics = table :periodics_tbl, [:pername] => [:period]
end

RSpec.describe('PosixClass parsing') do
  include_examples 'parse', /[[:word:]]/,
    [0]    => [CharacterSet, count: 1],
    [0, 0] => [:posixclass, :word, PosixClass, name: 'word', text: '[:word:]']
  include_examples 'parse', /[[:^word:]]/,
    [0]    => [CharacterSet, count: 1],
    [0, 0] => [:nonposixclass, :word, PosixClass, name: 'word', text: '[:^word:]']
end

# Spaces before a same-line heredoc opener are still offenses
let(:hiera_config) { <<~CONF }
---
version: 5
CONF

# Extra space in hash argument trailing a heredoc opener
self.post("/path", <<-XML, {"Content-Type" => "application/atom+xml"})
<?xml version='1.0' encoding='UTF-8'?>
<entry/>
XML

# Spaces before a chained `.` are still offenses for single-line receivers
data = { a: 1 } .transform_values
