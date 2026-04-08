# nitrocop-config: EnforcedStyle: use_builtin_english_names

# Builtin English names are allowed
$LOAD_PATH.unshift(lib) unless $LOAD_PATH.include?(lib)

puts $LOADED_FEATURES

puts $PROGRAM_NAME

# Perl names for non-builtins are allowed
puts $!

puts $$

puts $?

puts $~

puts $_

puts $;

puts $,

puts $/

puts $\

puts $.

puts $>

puts $<

puts $=

puts $*

puts $@

puts $+

puts $&

puts $`

# Regular globals are not flagged
puts $foo
