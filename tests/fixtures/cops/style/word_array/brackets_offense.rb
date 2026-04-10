# EnforcedStyle: brackets — %w/%W arrays should be converted to bracket syntax

# Simple %w array
%w[foo bar baz]
^ Style/WordArray: Use `['foo', 'bar', 'baz']` for an array of words.

# %w with parentheses delimiter
%w(http https)
^ Style/WordArray: Use `['http', 'https']` for an array of words.

# %W array
%W[foo bar baz]
^ Style/WordArray: Use `['foo', 'bar', 'baz']` for an array of words.

# In assignment context
x = %w[daily weekly]
    ^ Style/WordArray: Use `['daily', 'weekly']` for an array of words.

# Single element (no MinSize for brackets direction)
%w[single]
^ Style/WordArray: Use `['single']` for an array of words.

# Multi-line %w uses generic message
%w[
^ Style/WordArray: Use an array literal `[...]` for an array of words.
  foo
  bar
]

# Nested inside method call
validates :url, format: URI::Parser.new.make_regexp(%w(http https))
                                                    ^ Style/WordArray: Use `['http', 'https']` for an array of words.
