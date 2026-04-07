# nitrocop-config: EnforcedStyle: brackets
# (default)

hash.fetch(:key)
     ^^^^^^^^^^^^ Style/HashLookupMethod: Use `[]` instead of `fetch`.
hash.fetch('name')
     ^^^^^^^^^^^^^^ Style/HashLookupMethod: Use `[]` instead of `fetch`.
obj.fetch(x)
    ^^^^^^^^ Style/HashLookupMethod: Use `[]` instead of `fetch`.

block_given? ? identity.fetch(&block) : identity
                        ^^^^^^^^^^^^^ Style/HashLookupMethod: Use `[]` instead of `fetch`.
