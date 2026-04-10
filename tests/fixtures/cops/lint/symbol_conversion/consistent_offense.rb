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

# FN fix: undef inside a block body (preceded by {)
# fastlane pattern
FeatureHelper.instance_eval { undef test_method }
                                    ^^^^^^^^^^^ Lint/SymbolConversion: Unnecessary symbol conversion; use `:test_method` instead.

# FN fix: bare symbols whose source doesn't match correction should be flagged
# in consistent mode. Prism gives value "~" for :~@ (source :~@, correction :~).
# bloom-lang/bud and rubyworks/facets patterns.
method :~@
       ^^^ Lint/SymbolConversion: Unnecessary symbol conversion; use `:~` instead.

# FN fix: hash pattern in case/in should be processed like regular hash
# ruby-formatter/rufo pattern
case x
in { "a": 1 }
     ^^^^ Lint/SymbolConversion: Unnecessary symbol conversion; use `a:` instead.
  1
end
