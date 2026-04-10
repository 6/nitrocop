# nitrocop-expect: 1:0 Style/HashLookupMethod: Use `fetch` instead of `[]`.
hash[&block]
# nitrocop-expect: 2:0 Style/HashLookupMethod: Use `fetch` instead of `[]`.
hash[&-> { "foo" }]
# nitrocop-expect: 3:0 Style/HashLookupMethod: Use `fetch` instead of `[]`.
obj[&pr]
# nitrocop-expect: 4:0 Style/HashLookupMethod: Use `fetch` instead of `[]`.
hash[key]