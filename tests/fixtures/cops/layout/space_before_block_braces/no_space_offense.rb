# nitrocop-config: EnforcedStyle: no_space, EnforcedStyleForEmptyBraces: no_space
items.each { |x| puts x }
          ^ Layout/SpaceBeforeBlockBraces: Space detected to the left of {.
def m
  super { |x| puts x }
       ^ Layout/SpaceBeforeBlockBraces: Space detected to the left of {.
end
