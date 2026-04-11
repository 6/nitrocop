# nitrocop-config: EnforcedStyle: diff_comma

# Corpus FN: solargraph's multiline array inside a call still needs the final
# comma under `diff_comma`.
# nitrocop-expect: 10:46 Style/TrailingCommaInArrayLiteral: Put a comma after the last item of a multiline array.
formatter \
  SimpleCov::Formatter::MultiFormatter
    .new([
           SimpleCov::Formatter::HTMLFormatter,
           SimpleCov::Formatter::Undercover,
           SimpleCov::Formatter::LcovFormatter
         ])

# Corpus FN: betty's multiline array should also require the final comma.
# nitrocop-expect: 23:40 Style/TrailingCommaInArrayLiteral: Put a comma after the last item of a multiline array.
files = {
  examples: [
    "- betty copy folder my_songs/ to backup/",
    "- betty move folder my_songs/ to backup/",
    "- betty delete file junk.txt",
    "- betty remove file junk.txt",
    "- betty delete folder logs/",
    "- betty remove folder logs/",
    "- betty cleanup folder logs/",
    "- betty force cleanup folder logs/"
  ]
}
