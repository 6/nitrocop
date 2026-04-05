do_something(<<~EOS)
  no string interpolation style text
EOS

do_something(<<~'EOS')
  text with #{interpolation} patterns
EOS

do_something(<<~`EOS`)
  command
EOS

do_something(<<~'EOS')
  text with #@instance_var patterns
EOS

do_something(<<~'EOS')
  text with #$global_var patterns
EOS

do_something(<<~'EOS')
  Preserve \
  newlines
EOS

do_something(<<~"EDGE'CASE")
  no string interpolation style text
EDGE'CASE

module_eval(<<'.,.,', 'egrammar.ra', 67)
  def _reduce_1(val, _values, result)
    result
  end
.,.,

do_something(<<~'my-delimiter')
  text here
my-delimiter

do_something(<<~'has spaces')
  text here
has spaces

do_something(<<~'EDGE+CASE')
  text here
EDGE+CASE

do_something(<<~"EOS")
  text with #{interpolation} patterns
EOS

do_something(<<-"EOS")
  include #{code}
EOS

do_something(<<"EOS")
  text with #@instance_var patterns
EOS

do_something(<<~"EOS")
  text with #$global_var patterns
EOS

code = <<-"CODE"
1 2\n3
      CODE

code = <<-"CODE"
"x\\\\u{1f452}y"
      CODE

code = <<-"CODE"
# \u{0400}\na
      CODE

x = 1
y = 2

# Fuzz crash regression: malformed heredoc with unterminated quote (crash-be7c898f)
!<<"A
