# nitrocop-config: EnforcedStyle: no_space, EnforcedStyleForEmptyBraces: no_space
items.each{ |x| puts x }
items.each do |x|
  puts x
end
def m
  super{ |x| puts x }
end
