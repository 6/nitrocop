args.merge!(
  'key(1i)' => :value1,
  key: :value2
  ^ Style/HashSyntax: Don't mix styles in the same hash.
)

{ 'a' => 1, b: 2 }
            ^ Style/HashSyntax: Don't mix styles in the same hash.

test_gvar_LOAD_PATH(gvar: :$:)
                          ^^^ Style/HashSyntax: Include the hash value.
