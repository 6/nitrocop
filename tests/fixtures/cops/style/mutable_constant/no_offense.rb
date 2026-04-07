=begin
Copyright 2026 Nitrocop
=end
#
# frozen-string-literal: true

CONST = [].freeze

CONST2 = {}.freeze

CONST3 = "hello".freeze

CONST4 = 42

CONST5 = :symbol

# String constants are already frozen with the magic comment
CONST6 = 'web+mastodon'
CONST7 = "hello world"

# Numeric and symbol literals are immutable
CONST8 = 1.5

CONST9 = true

CONST10 = nil

# Regexp and range are frozen since Ruby 3.0
CONST11 = /regexp/
CONST12 = 1..99
CONST13 = 1...99
CONST14 = (1..99)

# Method calls are not mutable literals (in default 'literals' mode)
CONST15 = Something.new
CONST16 = "foo" + "bar"
CONST17 = FOO + BAR

# Backtick (xstring) with .freeze is not flagged
CONST18 = `uname`.freeze

# Plain strings stay allowed with long comment headers and hyphenated magic comments
CONST19 = "/"
CONST20 = "✓"
CONST21 = "\e[31m"

# Interpolated symbols with .freeze are immutable (strict mode)
# These were FP: nitrocop flagged them but RuboCop doesn't
THREAD_KEY = :"#{name}.batched_queries".freeze
ATTRIBUTE_NAMESPACE = :"job.#{base}".freeze
