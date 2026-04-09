items.each { | x | puts x }
items.each do | x |
  puts x
end
super do | klass, names, options |
  puts klass
end
super { | x | puts x }
->( x, y ) { puts x }
