# nitrocop-config: EnforcedStyle: no_space
# nitrocop-config: EnforcedStyleForEmptyBraces: space
items.map { |item|
  item.do_something
  }
^^ Layout/SpaceInsideBlockBraces: Space inside } detected.

foo {[
  bar
  ]}
^^ Layout/SpaceInsideBlockBraces: Space inside } detected.
