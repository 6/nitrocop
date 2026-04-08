# nitrocop-config: EnforcedStyle: use_builtin_english_names

# Perl names for builtins should suggest the English builtin
$:.push File.expand_path('lib', __dir__)
^ Style/SpecialGlobalVars: Prefer `$LOAD_PATH` over `$:`.

$" << "foo"
^ Style/SpecialGlobalVars: Prefer `$LOADED_FEATURES` over `$"`.

puts $0
     ^^ Style/SpecialGlobalVars: Prefer `$PROGRAM_NAME` over `$0`.

# Non-builtin English names should suggest the Perl equiv
puts $ERROR_INFO
     ^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$!` over `$ERROR_INFO`.

puts $PROCESS_ID
     ^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$$` over `$PROCESS_ID`.

puts $CHILD_STATUS
     ^^^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$?` over `$CHILD_STATUS`.

puts $LAST_MATCH_INFO
     ^^^^^^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$~` over `$LAST_MATCH_INFO`.

puts $LAST_READ_LINE
     ^^^^^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$_` over `$LAST_READ_LINE`.

puts $FIELD_SEPARATOR
     ^^^^^^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$;` over `$FIELD_SEPARATOR`.

puts $OUTPUT_FIELD_SEPARATOR
     ^^^^^^^^^^^^^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$,` over `$OUTPUT_FIELD_SEPARATOR`.

puts $INPUT_RECORD_SEPARATOR
     ^^^^^^^^^^^^^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$/` over `$INPUT_RECORD_SEPARATOR`.

puts $OUTPUT_RECORD_SEPARATOR
     ^^^^^^^^^^^^^^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$\` over `$OUTPUT_RECORD_SEPARATOR`.

puts $INPUT_LINE_NUMBER
     ^^^^^^^^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$.` over `$INPUT_LINE_NUMBER`.

puts $DEFAULT_OUTPUT
     ^^^^^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$>` over `$DEFAULT_OUTPUT`.

puts $DEFAULT_INPUT
     ^^^^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$<` over `$DEFAULT_INPUT`.

puts $ERROR_POSITION
     ^^^^^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$@` over `$ERROR_POSITION`.

puts $LAST_PAREN_MATCH
     ^^^^^^^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$+` over `$LAST_PAREN_MATCH`.

puts $MATCH
     ^^^^^^ Style/SpecialGlobalVars: Prefer `$&` over `$MATCH`.

puts $POSTMATCH
     ^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$'` over `$POSTMATCH`.

puts $PREMATCH
     ^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$`` over `$PREMATCH`.

puts $IGNORECASE
     ^^^^^^^^^^^ Style/SpecialGlobalVars: Prefer `$=` over `$IGNORECASE`.

puts $ARGV
     ^^^^^ Style/SpecialGlobalVars: Prefer `$*` over `$ARGV`.
