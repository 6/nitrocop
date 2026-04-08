# nitrocop-config: EnforcedStyle: consistent

# FN fix: undef arguments are bare symbols that should be flagged in consistent mode.
# RuboCop's properly_quoted? doesn't short-circuit for quote-less sources in consistent mode,
# so `is_a?` (source) != `:is_a?` (correction) → offense.

# Simple undef (CocoaPods pattern)
undef is_a?
      ^^^^^ Lint/SymbolConversion: Unnecessary symbol conversion; use `:is_a?` instead.

# undef with simple identifier (rack-mini-profiler pattern)
undef clock_gettime
      ^^^^^^^^^^^^^^ Lint/SymbolConversion: Unnecessary symbol conversion; use `:clock_gettime` instead.

# undef with multiple methods (comma-separated)
undef foo, bar
      ^^^ Lint/SymbolConversion: Unnecessary symbol conversion; use `:foo` instead.
           ^^^ Lint/SymbolConversion: Unnecessary symbol conversion; use `:bar` instead.
