# nitrocop-config: EnforcedStyle: brackets
%i[foo bar baz]
^ Style/SymbolArray: Use `[:foo, :bar, :baz]` for an array of symbols.

%i[one two]
^ Style/SymbolArray: Use `[:one, :two]` for an array of symbols.

%I[alpha beta]
^ Style/SymbolArray: Use `[:alpha, :beta]` for an array of symbols.

# Single element percent arrays should also be flagged in brackets mode
%i[solo]
^ Style/SymbolArray: Use `[:solo]` for an array of symbols.

# Empty percent arrays should also be flagged
%i[]
^ Style/SymbolArray: Use `[]` for an array of symbols.
