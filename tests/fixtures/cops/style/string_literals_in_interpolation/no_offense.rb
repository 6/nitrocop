"result is #{x == 'foo'}"
"hello #{hash['key']}"
"test #{y.gsub('a', 'b')}"
"plain string"
'single quoted'
x = "#{42}"
# Double-quoted strings with embedded single quotes need double quotes
"'#{elements.join("', '")}'"
# Strings inside %x() / backtick interpolation should not be flagged
%x( createdb #{config["arunit"]["database"]} )
%x( dropdb --if-exists #{config["arunit2"]["database"]} )
`#{File.expand_path("../../exe/rails", __dir__)} new --help`
`cd #{repo_dir} && git init . #{"--initial-branch=main" if supported}`
# Double-quoted strings with unrecognized escapes (meaning differs in single quotes)
"metric #{metric.gsub("\.", "\/")}"
# Double-quoted strings with escaped single quotes (\' is literal ' in dstr)
"target '#{host.name.gsub(/'/, "\\\\\'")}' do"
# Strings with \# escape (literal # in double quotes, \# in single quotes)
"#{CGI.escape("A&(! 234k !@ kasdj232\#$ kjw35")}"
# Strings directly inside backtick xstr interpolation are not flagged
`passbolt #{config["database"]} --json`
# RuboCop skips multiline quoted literals inside interpolation because parser emits dstr, not str
"#{"foo
bar"}"
"#{condition ? 'a' : "b
c"}"
"#{"sql
fragment" if enabled}"
