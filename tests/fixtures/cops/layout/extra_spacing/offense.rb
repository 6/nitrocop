set_app("RuboCop")
website  = "https://github.com/rubocop"
       ^ Layout/ExtraSpacing: Unnecessary spacing detected.

x  = 1
 ^ Layout/ExtraSpacing: Unnecessary spacing detected.

method_call(arg1,  arg2)
                 ^ Layout/ExtraSpacing: Unnecessary spacing detected.

# Alignment where adjacent token is NOT preceded by space (coincidental vertical alignment)
d_is_vertically_aligned  do
                       ^ Layout/ExtraSpacing: Unnecessary spacing detected.
  _______________________d
end

# Extra space before a float in multiline array
{:a => "a",
 :b => [nil,  2.5]}
            ^ Layout/ExtraSpacing: Unnecessary spacing detected.

# Extra spacing in class inheritance
class A   < String
       ^^ Layout/ExtraSpacing: Unnecessary spacing detected.
end

# Extra spacing before a unary plus in an argument list
assert_difference(MyModel.count, +2,
                  3,  +3,
                    ^ Layout/ExtraSpacing: Unnecessary spacing detected.
                  4,+4)

# Single-line hash with extra spaces
hash = {a:   1,  b:    2}
          ^^ Layout/ExtraSpacing: Unnecessary spacing detected.
               ^ Layout/ExtraSpacing: Unnecessary spacing detected.
                   ^^^ Layout/ExtraSpacing: Unnecessary spacing detected.

# Trailing comments at different columns - NOT aligned, should be flagged
check_a_pattern_result   # comment A
                      ^ Layout/ExtraSpacing: Unnecessary spacing detected.
check_b   # comment B
       ^ Layout/ExtraSpacing: Unnecessary spacing detected.
check_c_patterns   # comment C
                ^ Layout/ExtraSpacing: Unnecessary spacing detected.

# Extra spaces inside empty word arrays (RuboCop flags these)
a = %w(  )
       ^ Layout/ExtraSpacing: Unnecessary spacing detected.

# Extra space after assert (not aligned with anything meaningful)
assert  @fake_stderr.contained?(/flag/)
      ^ Layout/ExtraSpacing: Unnecessary spacing detected.
assert !@called

# Extra space after opening brace
{  portal: {
 ^ Layout/ExtraSpacing: Unnecessary spacing detected.
  name: 'test_portal'
} }

# Alignment FN: ||= with extra spaces not aligned with adjacent =
# (different last_column of = sign)
@signatures[pair_hash]      ||= {}
                      ^^^^^ Layout/ExtraSpacing: Unnecessary spacing detected.
@data_gathering[pair_hash] ||= {}

let(:output_missing) {      <<-EOT
                      ^^^^^ Layout/ExtraSpacing: Unnecessary spacing detected.
EOT
}

option.  #{ BlueHydra.config["file"] ? "\n\nReading data from " + BlueHydra.config["file"]  + '.' : '' }
       ^ Layout/ExtraSpacing: Unnecessary spacing detected.

assert { case1("@gptあ")   == "あ" }
                         ^^ Layout/ExtraSpacing: Unnecessary spacing detected.

[0, 0] => [:posixclass,    :word, PosixClass, name: 'word', text: '[:word:]']
                       ^^^ Layout/ExtraSpacing: Unnecessary spacing detected.

text str: 'The Title',   layout: :title # from custom-layout.yml
                      ^^ Layout/ExtraSpacing: Unnecessary spacing detected.

expected_out = Torch.tensor([
  [[ 0.7493,  0.4482, -2.1426,  0.5586,  0.5540, -0.1676],
                              ^ Layout/ExtraSpacing: Unnecessary spacing detected.
   [-1.7787,  1.3332, -0.3269, -0.2184,  0.9501,  0.0408]],

  [[ 0.0258, -0.3633,  0.4725, -0.5102,  1.8175, -1.4423],
                     ^ Layout/ExtraSpacing: Unnecessary spacing detected.
   [-0.8428,  0.8163, -1.7820,  0.9993,  0.1579,  0.6513]],
                              ^ Layout/ExtraSpacing: Unnecessary spacing detected.
])

html = <<-EOF
#{foo(1,  2)}
        ^ Layout/ExtraSpacing: Unnecessary spacing detected.
#{bar(3, 4)}
#{baz(5,  6)}
        ^ Layout/ExtraSpacing: Unnecessary spacing detected.
EOF

(%w[  id lock_version position version_comment created_at updated_at created_by_id updated_by_id type original_record_id])
    ^ Layout/ExtraSpacing: Unnecessary spacing detected.

def builtin_state
  raise Bud::Error unless @tables.empty?

  loopback  :localtick, [:col1]
          ^ Layout/ExtraSpacing: Unnecessary spacing detected.
  @stdio = terminal :stdio
  scratch :halt, [:key]
  @periodics = table :periodics_tbl, [:pername] => [:period]
end

RSpec.describe('PosixClass parsing') do
  include_examples 'parse', /[[:word:]]/,
    [0]    => [CharacterSet, count: 1],
    [0, 0] => [:posixclass,    :word, PosixClass, name: 'word', text: '[:word:]']
                           ^^^ Layout/ExtraSpacing: Unnecessary spacing detected.
  include_examples 'parse', /[[:^word:]]/,
    [0]    => [CharacterSet, count: 1],
    [0, 0] => [:nonposixclass, :word, PosixClass, name: 'word', text: '[:^word:]']
end

# Spaces before a same-line heredoc opener are still offenses
let(:hiera_config) {      <<~CONF }
                    ^^^^^ Layout/ExtraSpacing: Unnecessary spacing detected.
---
version: 5
CONF

# Extra space in hash argument trailing a heredoc opener
self.post("/path", <<-XML, {"Content-Type" =>  "application/atom+xml"})
                                             ^ Layout/ExtraSpacing: Unnecessary spacing detected.
<?xml version='1.0' encoding='UTF-8'?>
<entry/>
XML

# Spaces before a chained `.` are still offenses for single-line receivers
data = { a: 1 }  .transform_values
               ^ Layout/ExtraSpacing: Unnecessary spacing detected.
