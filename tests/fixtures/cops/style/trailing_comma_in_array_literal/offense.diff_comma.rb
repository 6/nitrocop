# nitrocop-config: EnforcedStyle: diff_comma
formatter \
  SimpleCov::Formatter::MultiFormatter
    .new([
           SimpleCov::Formatter::HTMLFormatter,
           SimpleCov::Formatter::Undercover,
           SimpleCov::Formatter::LcovFormatter
                                              ^ Style/TrailingCommaInArrayLiteral: Put a comma after the last item of a multiline array.
         ])

x = [
  "- betty copy file my.txt to usr/",
  "- betty move file my.txt to usr/",
  "- betty force cleanup folder logs/"
                                      ^ Style/TrailingCommaInArrayLiteral: Put a comma after the last item of a multiline array.
]
